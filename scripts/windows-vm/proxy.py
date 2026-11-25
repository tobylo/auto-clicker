#!/usr/bin/env python3
"""Allowlisting HTTP proxy for the Windows test VM, one connection per process.

QEMU starts this script for each TCP connection the VM opens to the proxy
address (a `guestfwd ... cmd:` rule), with the connection on stdin and stdout,
so nothing listens on the dev box. It serves one request:

- `CONNECT host:443`, then relays bytes both ways (HTTPS);
- an absolute-form `GET http://host/...` (or HEAD) to port 80 with no body,
  sent on with `Connection: close`, then relays the response (plain HTTP,
  which Windows Update and certificate revocation checks use).

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
import threading
import time

CONNECT_TIMEOUT = 15
DNS_TIMEOUT = 15
HEADER_TIMEOUT = 30
IDLE_TIMEOUT = 300
MAX_HEADER_BYTES = 64 * 1024
MAX_REQUEST_LINE = 8 * 1024
MAX_LOGGED_CHARS = 300
CHUNK = 64 * 1024
PORTS = {"CONNECT": 443, "GET": 80, "HEAD": 80}
# A DNS name in letters, digits and hyphens, with an optional final dot:
# nothing the resolver could read differently from the allowlist check (a
# NUL, say, after which it stops reading).
# A request line as the proxy rebuilds it. Neither it nor a header line
# (below) may hold a bare CR or LF, which some servers read as a line end.
REQUEST_LINE = re.compile(r"[A-Z]+ [!-~]+ HTTP/1\.[01]\Z")
# A header line in its one plain form: a token name directly followed by a
# colon. A space before the colon or a folded line is read one way here and
# another way by some servers, so it is refused rather than interpreted.
HEADER_LINE = re.compile(r"[!#$%&'*+.^_`|~0-9A-Za-z-]+:[^\r\n]*\Z")
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


def resolve(host, port):
    """getaddrinfo within DNS_TIMEOUT. The lookup itself cannot be cancelled,
    so it runs in a daemon thread that ends with this one-request process."""
    outcome = {}

    def lookup():
        try:
            outcome["infos"] = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
        except OSError as err:
            outcome["error"] = err

    worker = threading.Thread(target=lookup, daemon=True)
    worker.start()
    worker.join(DNS_TIMEOUT)
    if worker.is_alive():
        raise Refused(504, f"lookup of {host} took over {DNS_TIMEOUT} s")
    if "error" in outcome:
        raise outcome["error"]
    return outcome["infos"]


def global_addresses(host, port):
    """The host's global addresses; private, loopback and link-local ones are dropped."""
    infos = resolve(host, port)
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


def recv_by(conn, deadline):
    """One read before `deadline` (a monotonic time); refuses a connection
    that closes before the header ended."""
    left = deadline - time.monotonic()
    if left <= 0:
        raise TimeoutError("the request header did not arrive in time")
    conn.settimeout(left)
    chunk = conn.recv(CHUNK)
    if not chunk:
        raise Refused(400, "connection closed before the header ended")
    return chunk


def read_header(conn):
    """Reads up to the blank line ending the request header; returns (head, rest)."""
    # One deadline for the whole header, so a trickle of bytes cannot extend it.
    deadline = time.monotonic() + HEADER_TIMEOUT
    data = b""
    while True:
        data += recv_by(conn, deadline)
        head, end, rest = data.partition(b"\r\n\r\n")
        if len(head) > MAX_HEADER_BYTES:
            raise Refused(431, "request header too large")
        if end:
            return checked_head(head.decode("latin-1")), rest


def checked_head(head):
    """The header text, refused unless its line breaks and request line are plain."""
    line = head.split("\r\n", 1)[0]
    if len(line) > MAX_REQUEST_LINE:
        raise Refused(414, "request line too long")
    if not REQUEST_LINE.match(line):
        raise Refused(400, "malformed request line or header")
    if not all(HEADER_LINE.match(field) for field in head.split("\r\n")[1:]):
        raise Refused(400, "header line not in plain form")
    return head


def header_values(head, name):
    """Every value of header `name` in the header text."""
    values = []
    for line in head.split("\r\n")[1:]:
        key, _, value = line.partition(":")
        if key.lower() == name:
            values.append(value.strip())
    return values


def check_bodiless(head, rest):
    """Refuses a plain-HTTP request that sends or announces a body. GET and
    HEAD carry none, so whatever follows the header would be a second
    request, whose Host the proxy never checked."""
    if rest:
        raise Refused(400, "bytes after the request header")
    announced = header_values(head, "transfer-encoding") + header_values(head, "expect")
    if announced or set(header_values(head, "content-length")) - {"0"}:
        raise Refused(400, "a body announced on a request without one")


DROPPED_HEADERS = ("host", "connection", "proxy-connection", "keep-alive")


def connection_options(head):
    """The header names the Connection header lists: they are meant for the
    proxy alone and go no further (RFC 9110, 7.6.1)."""
    return {
        name.strip().lower()
        for value in header_values(head, "connection")
        for name in value.split(",")
    }


def origin_request(head, target, host, port):
    """Rewrites an absolute-form request to origin form for the checked host,
    closing after one response. The VM's own Host header is dropped: it could
    name another site behind the same server."""
    lines = head.split("\r\n")
    method, _, version = lines[0].split(" ", 2)
    path = "/" + target.split("://", 1)[1].partition("/")[2]
    dropped = set(DROPPED_HEADERS) | connection_options(head)
    kept = [line for line in lines[1:] if line.split(":", 1)[0].lower() not in dropped]
    authority = host if port == 80 else f"{host}:{port}"
    out = [f"{method} {path} {version}", f"Host: {authority}", *kept, "Connection: close", "", ""]
    return "\r\n".join(out).encode("latin-1")


def connect_upstream(addresses):
    """Connects to the first address that answers within the deadline."""
    last_error = None
    for family, sockaddr in addresses:
        try:
            return connect_one(family, sockaddr)
        except OSError as err:
            last_error = err
    raise Refused(502, f"cannot connect: {last_error}")


def connect_one(family, sockaddr):
    """A socket connected to `sockaddr` within CONNECT_TIMEOUT. Creating it
    can fail too, for a family the dev box has turned off."""
    sock = socket.socket(family, socket.SOCK_STREAM)
    try:
        sock.settimeout(CONNECT_TIMEOUT)
        sock.connect(sockaddr)
    except OSError:
        sock.close()
        raise
    return sock


def relay(client, upstream, sources=None):
    """Copies bytes from each of `sources` (both sides unless given) to the
    other side until every source has closed, or none has sent anything for
    IDLE_TIMEOUT. A side that closes only its sending half still gets the
    rest of the other side's answer."""
    selector = selectors.DefaultSelector()
    peers = {client: upstream, upstream: client}
    for sock in peers:
        sock.setblocking(False)
    for sock in sources or peers:
        selector.register(sock, selectors.EVENT_READ)
    try:
        while selector.get_map() and forward_ready(selector, selector.select(timeout=IDLE_TIMEOUT), peers):
            pass
    finally:
        selector.close()


def half_close(selector, sock, out):
    """`sock` sent its last byte: stop reading it and tell `out` so."""
    selector.unregister(sock)
    try:
        out.shutdown(socket.SHUT_WR)
    except OSError:
        pass  # `out` has gone already; its own end shows on its next read


def forward_ready(selector, events, peers):
    """Forwards what each ready socket has; False once both stayed idle."""
    if not events:
        return False
    for key, _ in events:
        data = key.fileobj.recv(CHUNK)
        out = peers[key.fileobj]
        if not data:
            half_close(selector, key.fileobj, out)
            continue
        out.setblocking(True)
        out.settimeout(IDLE_TIMEOUT)
        out.sendall(data)
        out.setblocking(False)
    return True


def log(logfile, message):
    """Appends one line; text from the VM is cut short, so the VM cannot
    grow the log by more than a line per connection."""
    if len(message) > MAX_LOGGED_CHARS:
        message = message[:MAX_LOGGED_CHARS] + "..."
    stamp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    with open(logfile, "a", encoding="utf-8") as handle:
        handle.write(f"{stamp} {message}\n")


def serve(conn, allowlist, logfile):
    """Serves the one request on `conn`."""
    try:
        head, rest = read_header(conn)
        method, target = head.split("\r\n", 1)[0].split(" ")[:2]
        host, port = parse_target(method, target)
        if method != "CONNECT":
            check_bodiless(head, rest)
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
            upstream.sendall(rest)
            relay(conn, upstream)
        else:
            # One request; anything the VM sends after it is never read.
            upstream.sendall(origin_request(head, target, host, port))
            relay(conn, upstream, sources=(upstream,))


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
