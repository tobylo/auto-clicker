#!/usr/bin/env python3
"""Allowlisting HTTP proxy for the Windows test VM, one connection per process.

QEMU starts this script for each TCP connection the VM opens to the proxy
address (a `guestfwd ... cmd:` rule), with the connection on stdin and stdout,
so nothing listens on the dev box. It serves one request:

- `CONNECT host:443`, then relays bytes both ways (HTTPS);
- an absolute-form `GET http://host/...` (or HEAD, POST) to port 80, sent on
  with `Connection: close`, then relays the response (plain HTTP, which
  Windows Update and certificate revocation checks use).

A host passes only when the allowlist file names it, and its address is
connected to only when it is a global address: never the LAN, loopback,
link-local or any other private range, whatever DNS answers. Each request is
appended to the log file, passed or refused, so the log shows what the VM
reached and a failed build names the host it needed.

Usage: proxy.py LOGFILE ALLOWLIST [ALLOWLIST...]
"""

import ipaddress
import os
import re
import selectors
import socket
import sys
import time

CONNECT_TIMEOUT = 15
HEADER_TIMEOUT = 30
IDLE_TIMEOUT = 300
MAX_HEADER_BYTES = 64 * 1024
CHUNK = 64 * 1024
PORTS = {"CONNECT": 443, "GET": 80, "HEAD": 80, "POST": 80}
# A DNS name in letters, digits and hyphens, with an optional final dot:
# nothing the resolver could read differently from the allowlist check (a
# NUL, say, after which it stops reading).
# A request line as the proxy rebuilds it, and header text with no line break
# but CRLF: a bare LF or CR is read as a line end by some servers and not by
# others, so it could carry a second Host past the one the proxy writes.
REQUEST_LINE = re.compile(r"[A-Z]+ [!-~]+ HTTP/1\.[01]\Z")
BARE_BREAK = re.compile(r"\r(?!\n)|(?<!\r)\n")
DNS_NAME = re.compile(r"(?=.{1,253}\.?\Z)[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)*\.?\Z")


class Refused(Exception):
    """A request the proxy turns away, with the status to answer."""

    def __init__(self, status, reason):
        super().__init__(reason)
        self.status = status
        self.reason = reason


def load_allowlist(paths):
    """Reads the allowlists: one host per line; a leading dot also allows subdomains."""
    entries = set()
    for path in paths:
        with open(path, encoding="ascii") as handle:
            for raw in handle:
                line = raw.split("#", 1)[0].strip().lower()
                if line:
                    entries.add(line)
    return entries


def host_allowed(host, allowlist):
    """Whether `host` is listed itself, or sits under a listed `.domain`."""
    host = host.lower().rstrip(".")
    if host in allowlist:
        return True
    parts = host.split(".")
    return any("." + ".".join(parts[i:]) in allowlist for i in range(len(parts)))


def is_ip_literal(host):
    try:
        ipaddress.ip_address(host)
    except ValueError:
        return False
    return True


def global_addresses(host, port):
    """The host's global addresses; private, loopback and link-local ones are dropped."""
    infos = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    found = []
    for family, _, _, _, sockaddr in infos:
        if ipaddress.ip_address(sockaddr[0]).is_global:
            found.append((family, sockaddr))
    return found


def request_refusal(method, host, port, allowlist):
    """Why the VM may not use this method, port or host name; None when it may."""
    if method not in PORTS:
        return 405, f"method {method}"
    rules = (
        (port != PORTS[method], f"port {port} for {method}"),
        (is_ip_literal(host), f"address literal {host}"),
        (not DNS_NAME.match(host.lower()), f"not a host name: {host!r}"),
        (not host_allowed(host, allowlist), f"host {host} not listed"),
    )
    for broken, reason in rules:
        if broken:
            return 403, reason
    return None


def check_request(method, host, port, allowlist):
    """Refuses a method, port or host name the VM may not use."""
    refusal = request_refusal(method, host, port, allowlist)
    if refusal:
        raise Refused(*refusal)


def check_target(method, host, port, allowlist):
    """Refuses a target the VM may not reach, else returns where to connect."""
    check_request(method, host, port, allowlist)
    try:
        addresses = global_addresses(host, port)
    except OSError as err:
        raise Refused(502, f"cannot resolve {host}: {err}") from err
    if not addresses:
        raise Refused(403, f"host {host} has no global address")
    return addresses


def split_host_port(authority, default_port):
    """Splits `host[:port]`; raises Refused on a malformed authority."""
    host, sep, port_text = authority.rpartition(":")
    if not sep or "]" in port_text:
        return authority.strip("[]"), default_port
    if not port_text.isdigit():
        raise Refused(400, f"bad port in {authority!r}")
    return host.strip("[]"), int(port_text)


def parse_target(method, target):
    """The host and port a request line names."""
    if method == "CONNECT":
        return split_host_port(target, 443)
    prefix = "http://"
    if not target.lower().startswith(prefix):
        raise Refused(400, "only absolute http:// targets")
    authority = target[len(prefix):].split("/", 1)[0]
    return split_host_port(authority, 80)


def read_header(conn):
    """Reads up to the blank line ending the request header; returns (head, rest)."""
    # One deadline for the whole header, so a trickle of bytes cannot extend it.
    deadline = time.monotonic() + HEADER_TIMEOUT
    data = b""
    while b"\r\n\r\n" not in data:
        if len(data) > MAX_HEADER_BYTES:
            raise Refused(431, "request header too large")
        left = deadline - time.monotonic()
        if left <= 0:
            raise TimeoutError("the request header did not end in time")
        conn.settimeout(left)
        chunk = conn.recv(CHUNK)
        if not chunk:
            raise Refused(400, "connection closed before the header ended")
        data += chunk
    head, _, rest = data.partition(b"\r\n\r\n")
    return checked_head(head.decode("latin-1")), rest


def checked_head(head):
    """The header text, refused unless its line breaks and request line are plain."""
    if BARE_BREAK.search(head) or not REQUEST_LINE.match(head.split("\r\n", 1)[0]):
        raise Refused(400, "malformed request line or header")
    return head


DROPPED_HEADERS = ("host", "connection", "proxy-connection", "keep-alive")


def origin_request(head, target, host, port):
    """Rewrites an absolute-form request to origin form for the checked host,
    closing after one response. The VM's own Host header is dropped: it could
    name another site behind the same server."""
    lines = head.split("\r\n")
    method, _, version = lines[0].split(" ", 2)
    path = "/" + target.split("://", 1)[1].partition("/")[2]
    kept = [line for line in lines[1:] if line.split(":", 1)[0].strip().lower() not in DROPPED_HEADERS]
    authority = host if port == 80 else f"{host}:{port}"
    out = [f"{method} {path} {version}", f"Host: {authority}", *kept, "Connection: close", "", ""]
    return "\r\n".join(out).encode("latin-1")


def connect_upstream(addresses):
    """Connects to the first address that answers within the deadline."""
    last_error = None
    for family, sockaddr in addresses:
        sock = socket.socket(family, socket.SOCK_STREAM)
        sock.settimeout(CONNECT_TIMEOUT)
        try:
            sock.connect(sockaddr)
            return sock
        except OSError as err:
            last_error = err
            sock.close()
    raise Refused(502, f"cannot connect: {last_error}")


def relay(client, upstream):
    """Copies bytes both ways until either side closes or both stay idle too long."""
    selector = selectors.DefaultSelector()
    peers = {client: upstream, upstream: client}
    for sock in peers:
        sock.setblocking(False)
        selector.register(sock, selectors.EVENT_READ)
    try:
        while forward_ready(selector.select(timeout=IDLE_TIMEOUT), peers):
            pass
    finally:
        selector.close()


def forward_ready(events, peers):
    """Forwards what each ready socket has; False once idle or one side has closed."""
    if not events:
        return False
    for key, _ in events:
        data = key.fileobj.recv(CHUNK)
        if not data:
            return False
        out = peers[key.fileobj]
        out.setblocking(True)
        out.settimeout(IDLE_TIMEOUT)
        out.sendall(data)
        out.setblocking(False)
    return True


def log(logfile, message):
    stamp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    with open(logfile, "a", encoding="utf-8") as handle:
        handle.write(f"{stamp} {message}\n")


def serve(conn, allowlist, logfile):
    """Serves the one request on `conn`."""
    try:
        head, rest = read_header(conn)
        method, target = head.split("\r\n", 1)[0].split(" ")[:2]
        host, port = parse_target(method, target)
        addresses = check_target(method, host, port, allowlist)
        upstream = connect_upstream(addresses)
    except Refused as refusal:
        log(logfile, f"refused {refusal.status}: {refusal.reason}")
        conn.sendall(f"HTTP/1.1 {refusal.status} Refused\r\nConnection: close\r\n\r\n".encode())
        return
    except ValueError:
        log(logfile, "refused 400: malformed request line")
        conn.sendall(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
        return
    log(logfile, f"passed {method} {host}:{port}")
    with upstream:
        if method == "CONNECT":
            conn.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
        else:
            upstream.sendall(origin_request(head, target, host, port))
        if rest:
            upstream.sendall(rest)
        relay(conn, upstream)


def main(argv):
    if len(argv) < 3:
        print("usage: proxy.py LOGFILE ALLOWLIST [ALLOWLIST...]", file=sys.stderr)
        return 2
    logfile = argv[1]
    allowlist = load_allowlist(argv[2:])
    with socket.socket(fileno=os.dup(0)) as conn:
        try:
            serve(conn, allowlist, logfile)
        except OSError as err:
            log(logfile, f"connection ended: {err}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
