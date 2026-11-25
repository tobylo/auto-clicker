"""Tests for proxy.py: what the VM may reach and what it is refused.

Run: python3 -I -m unittest discover -s scripts/windows-vm -p 'test_*.py'
"""

import os
import socket
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proxy  # noqa: E402

ALLOW = {"static.rust-lang.org", ".windowsupdate.com"}
PUBLIC = [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443))]


def receive(sock, count, seconds=10):
    """Reads exactly `count` bytes, however the stream splits them, or fails."""
    sock.settimeout(seconds)
    data = b""
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            break
        data += chunk
    return data


def receive_until(sock, ending, seconds=10):
    """Reads until the data ends with `ending`, or fails at the deadline."""
    sock.settimeout(seconds)
    data = b""
    while not data.endswith(ending):
        chunk = sock.recv(4096)
        if not chunk:
            break
        data += chunk
    return data


def resolving_to(*addresses):
    infos = [(socket.AF_INET, socket.SOCK_STREAM, 6, "", (a, 443)) for a in addresses]
    return mock.patch.object(proxy.socket, "getaddrinfo", return_value=infos)


class HostAllowed(unittest.TestCase):
    def test_listed_host_passes(self):
        self.assertTrue(proxy.host_allowed("static.rust-lang.org", ALLOW))

    def test_subdomain_passes_only_under_a_dot_entry(self):
        self.assertTrue(proxy.host_allowed("download.windowsupdate.com", ALLOW))
        self.assertTrue(proxy.host_allowed("windowsupdate.com", ALLOW))
        self.assertFalse(proxy.host_allowed("x.static.rust-lang.org", ALLOW))

    def test_names_the_resolver_could_read_differently_are_refused(self):
        # getaddrinfo stops at a NUL and would connect to the address before it.
        for host in ("93.184.216.34\x00.windowsupdate.com", "a b.windowsupdate.com", "x_y.windowsupdate.com",
                     "-x.windowsupdate.com", ".windowsupdate.com", "a..windowsupdate.com"):
            with self.subTest(host=host), resolving_to("93.184.216.34"), self.assertRaises(proxy.Refused) as caught:
                proxy.check_target("CONNECT", host, 443, ALLOW)
            self.assertIn("not a host name", caught.exception.reason)

    def test_lookalike_suffix_is_refused(self):
        self.assertFalse(proxy.host_allowed("evilwindowsupdate.com", ALLOW))
        self.assertFalse(proxy.host_allowed("static.rust-lang.org.evil.net", ALLOW))

    def test_case_and_trailing_dot_do_not_matter(self):
        self.assertTrue(proxy.host_allowed("Static.Rust-Lang.org.", ALLOW))


class CheckTarget(unittest.TestCase):
    def test_listed_host_with_global_address_is_returned(self):
        with resolving_to("93.184.216.34"):
            found = proxy.check_target("CONNECT", "static.rust-lang.org", 443, ALLOW)
        self.assertEqual(found, [(socket.AF_INET, ("93.184.216.34", 443))])

    def test_unlisted_host_is_refused_before_any_lookup(self):
        with mock.patch.object(proxy.socket, "getaddrinfo") as lookup:
            with self.assertRaises(proxy.Refused) as caught:
                proxy.check_target("CONNECT", "example.com", 443, ALLOW)
        self.assertEqual(caught.exception.status, 403)
        lookup.assert_not_called()

    def test_address_literals_are_refused(self):
        # A global literal too, listed and resolving to itself, so only the
        # literal check stands between it and a connection.
        for host in ("127.0.0.1", "10.0.2.2", "192.168.1.253", "::1", "93.184.216.34"):
            with self.subTest(host=host), resolving_to("93.184.216.34"), self.assertRaises(proxy.Refused) as caught:
                proxy.check_target("CONNECT", host, 443, ALLOW | {host})
            self.assertIn("address literal", caught.exception.reason)

    def test_listed_host_resolving_to_private_addresses_is_refused(self):
        for address in ("127.0.0.1", "192.168.1.253", "10.0.2.2", "169.254.1.1"):
            with self.subTest(address=address), resolving_to(address):
                with self.assertRaises(proxy.Refused):
                    proxy.check_target("CONNECT", "static.rust-lang.org", 443, ALLOW)

    def test_private_answers_are_dropped_beside_global_ones(self):
        with resolving_to("192.168.1.253", "93.184.216.34"):
            found = proxy.check_target("CONNECT", "static.rust-lang.org", 443, ALLOW)
        self.assertEqual([sockaddr[0] for _, sockaddr in found], ["93.184.216.34"])

    def test_wrong_port_for_method_is_refused(self):
        for method, port in (("CONNECT", 22), ("GET", 443)):
            with self.subTest(method=method), resolving_to("93.184.216.34"):
                with self.assertRaises(proxy.Refused) as caught:
                    proxy.check_target(method, "static.rust-lang.org", port, ALLOW)
                self.assertIn(f"port {port}", caught.exception.reason)

    def test_other_methods_are_refused(self):
        for method in ("POST", "PUT"):
            with self.subTest(method=method), self.assertRaises(proxy.Refused) as caught:
                proxy.check_target(method, "static.rust-lang.org", 80, ALLOW)
            self.assertEqual(caught.exception.status, 405)


class ParseTarget(unittest.TestCase):
    def test_connect_authority(self):
        self.assertEqual(proxy.parse_target("CONNECT", "a.b:443"), ("a.b", 443))

    def test_absolute_http_target(self):
        self.assertEqual(proxy.parse_target("GET", "http://a.b/x/y"), ("a.b", 80))
        self.assertEqual(proxy.parse_target("GET", "http://a.b:8080/"), ("a.b", 8080))

    def test_origin_form_is_refused(self):
        with self.assertRaises(proxy.Refused):
            proxy.parse_target("GET", "/x")

    def test_bad_port_is_refused(self):
        with self.assertRaises(proxy.Refused):
            proxy.parse_target("CONNECT", "a.b:https")


class OriginRequest(unittest.TestCase):
    def test_rewrites_to_origin_form_and_closes(self):
        head = "GET http://a.b/p?q=1 HTTP/1.1\r\nHost: a.b\r\nProxy-Connection: keep-alive\r\nConnection: keep-alive"
        out = proxy.origin_request(head, "http://a.b/p?q=1", "a.b", 80).decode()
        self.assertTrue(out.startswith("GET /p?q=1 HTTP/1.1\r\n"))
        self.assertIn("Host: a.b\r\n", out)
        self.assertNotIn("keep-alive", out.lower())
        self.assertTrue(out.endswith("Connection: close\r\n\r\n"))


    def test_host_header_is_the_checked_host_only(self):
        head = "GET http://a.b/ HTTP/1.1\r\nHost: unlisted.example\r\nhost: other.example\r\nAccept: */*"
        out = proxy.origin_request(head, "http://a.b/", "a.b", 80).decode()
        self.assertEqual([line for line in out.split("\r\n") if line.lower().startswith("host:")], ["Host: a.b"])
        self.assertIn("Accept: */*\r\n", out)

    def test_headers_named_by_connection_go_no_further(self):
        head = (
            "GET http://a.b/ HTTP/1.1\r\nConnection: keep-alive, X-Local\r\nconnection: x-other\r\n"
            "X-Local: proxy-only\r\nx-other: hop\r\nAccept: */*"
        )
        out = proxy.origin_request(head, "http://a.b/", "a.b", 80).decode()
        self.assertNotIn("x-local", out.lower())
        self.assertNotIn("x-other", out.lower())
        self.assertIn("Accept: */*\r\n", out)
        self.assertTrue(out.endswith("Connection: close\r\n\r\n"))


class Serve(unittest.TestCase):
    """One request through serve(), with the VM's side on a socket pair."""

    def setUp(self):
        self.vm, self.conn = socket.socketpair()
        self.addCleanup(self.vm.close)
        self.addCleanup(self.conn.close)
        handle, self.logfile = tempfile.mkstemp()
        os.close(handle)
        self.addCleanup(os.unlink, self.logfile)

    def serve(self, request):
        self.vm.sendall(request)
        proxy.serve(self.conn, ALLOW, self.logfile)
        self.conn.shutdown(socket.SHUT_RDWR)
        return self.vm.recv(4096)

    def test_unlisted_host_gets_403_and_is_logged(self):
        answer = self.serve(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n")
        self.assertTrue(answer.startswith(b"HTTP/1.1 403"))
        with open(self.logfile, encoding="utf-8") as handle:
            self.assertIn("example.com not listed", handle.read())

    def test_header_without_end_gets_431_once_over_the_cap(self):
        line = b"X-Fill: " + b"a" * 1000 + b"\r\n"
        self.vm.sendall(b"CONNECT static.rust-lang.org:443 HTTP/1.1\r\n")
        sender = threading.Thread(target=self.fill, args=(line, proxy.MAX_HEADER_BYTES // len(line) + 2))
        sender.start()
        proxy.serve(self.conn, ALLOW, self.logfile)
        sender.join(timeout=10)
        self.assertTrue(self.vm.recv(4096).startswith(b"HTTP/1.1 431"))

    def test_whole_header_over_the_cap_gets_431(self):
        fill = b"X-Fill: " + b"a" * (proxy.MAX_HEADER_BYTES + 1) + b"\r\n\r\n"
        sender = threading.Thread(target=self.vm.sendall, args=(b"CONNECT static.rust-lang.org:443 HTTP/1.1\r\n" + fill,))
        sender.start()
        with self.assertRaises(proxy.Refused) as caught:
            proxy.read_header(self.conn)
        sender.join(timeout=10)
        self.assertEqual(caught.exception.status, 431)

    def fill(self, line, count):
        for _ in range(count):
            self.vm.sendall(line)

    def test_bare_line_breaks_and_odd_request_lines_get_400(self):
        for request in (
            b"GET http://static.rust-lang.org/ HTTP/1.1\nHost: unlisted.example\n\n\r\n\r\n",
            b"GET http://static.rust-lang.org/ HTTP/1.1\r\nHost: a\rHost: unlisted.example\r\n\r\n",
            b"GET http://static.rust-lang.org/ HTTP/1.1\nX: y\r\n\r\n",
            b"GET http://static.rust-lang.org/ HTTP/9\r\n\r\n",
            b"GET  http://static.rust-lang.org/ HTTP/1.1\r\n\r\n",
            b"GET http://static.rust-lang.org/ HTTP/1.1\r\nContent-Length : 5\r\n\r\n",
            b"GET http://static.rust-lang.org/ HTTP/1.1\r\nContent-Length\xa0: 5\r\n\r\n",
            b"GET http://static.rust-lang.org/ HTTP/1.1\r\nAccept: */*\r\n Content-Length: 5\r\n\r\n",
        ):
            with self.subTest(request=request):
                vm, conn = socket.socketpair()
                self.addCleanup(vm.close)
                self.addCleanup(conn.close)
                vm.sendall(request)
                with self.assertRaises(proxy.Refused) as caught:
                    proxy.read_header(conn)
                self.assertEqual(caught.exception.status, 400)

    def test_second_request_behind_a_plain_http_one_gets_400(self):
        refuse = mock.patch.object(proxy, "connect_upstream", side_effect=AssertionError("connected upstream"))
        with resolving_to("93.184.216.34"), refuse as connect:
            answer = self.serve(
                b"GET http://static.rust-lang.org/ HTTP/1.1\r\n\r\nGET / HTTP/1.1\r\nHost: unlisted.example\r\n\r\n"
            )
        self.assertTrue(answer.startswith(b"HTTP/1.1 400"))
        connect.assert_not_called()

    def test_malformed_request_line_gets_400(self):
        answer = self.serve(b"garbage\r\n\r\n")
        self.assertTrue(answer.startswith(b"HTTP/1.1 400"))

    def test_connect_relays_both_ways(self):
        upstream_vm_side, upstream = socket.socketpair()
        self.addCleanup(upstream_vm_side.close)
        echo = threading.Thread(target=self.echo_once, args=(upstream_vm_side,), daemon=True)
        echo.start()
        with resolving_to("93.184.216.34"), mock.patch.object(proxy, "connect_upstream", return_value=upstream):
            self.vm.sendall(b"CONNECT static.rust-lang.org:443 HTTP/1.1\r\n\r\nping")
            worker = threading.Thread(target=proxy.serve, args=(self.conn, ALLOW, self.logfile), daemon=True)
            worker.start()
            expected = b"HTTP/1.1 200 Connection established\r\n\r\nping"
            self.assertEqual(receive(self.vm, len(expected)), expected)
            self.vm.shutdown(socket.SHUT_WR)
            self.assertEqual(receive(self.vm, 1), b"", "the upstream's close did not reach the VM")
            worker.join(timeout=10)
        echo.join(timeout=10)
        self.assertFalse(worker.is_alive(), "relay did not end when the VM closed")
        with open(self.logfile, encoding="utf-8") as handle:
            self.assertIn("passed CONNECT static.rust-lang.org:443", handle.read())


    @staticmethod
    def echo_once(sock):
        sock.sendall(sock.recv(64))
        sock.shutdown(socket.SHUT_WR)

    def test_tunnel_answer_still_arrives_after_the_vm_closes_its_sending_half(self):
        upstream_side, upstream = socket.socketpair()
        self.addCleanup(upstream_side.close)

        def answer_after_eof():
            request = b""
            while chunk := upstream_side.recv(4096):
                request += chunk
            upstream_side.sendall(b"answer to " + request)
            upstream_side.shutdown(socket.SHUT_WR)

        server = threading.Thread(target=answer_after_eof, daemon=True)
        server.start()
        with resolving_to("93.184.216.34"), mock.patch.object(proxy, "connect_upstream", return_value=upstream):
            self.vm.sendall(b"CONNECT static.rust-lang.org:443 HTTP/1.1\r\n\r\nhello")
            self.vm.shutdown(socket.SHUT_WR)
            worker = threading.Thread(target=proxy.serve, args=(self.conn, ALLOW, self.logfile), daemon=True)
            worker.start()
            expected = b"HTTP/1.1 200 Connection established\r\n\r\nanswer to hello"
            self.assertEqual(receive(self.vm, len(expected)), expected)
            worker.join(timeout=10)
        server.join(timeout=10)
        self.assertFalse(worker.is_alive(), "relay did not end once both sides closed")


class Bodiless(unittest.TestCase):
    """Plain HTTP carries GET and HEAD without a body, and nothing after
    them: a second request would carry an unchecked Host."""

    HEAD = "GET http://a.b/ HTTP/1.1\r\nHost: a.b"

    def test_request_without_a_body_passes(self):
        proxy.check_bodiless(self.HEAD, b"")
        proxy.check_bodiless(self.HEAD + "\r\nContent-Length: 0", b"")

    def test_bytes_after_the_header_are_refused(self):
        with self.assertRaises(proxy.Refused) as caught:
            proxy.check_bodiless(self.HEAD, b"GET / HTTP/1.1\r\nHost: unlisted.example\r\n\r\n")
        self.assertIn("bytes after the request header", caught.exception.reason)

    def test_a_body_announced_in_any_header_is_refused(self):
        for extra in (
            "Content-Length: 4",
            "Content-Length: 0\r\nContent-Length: 4",
            "Content-Length: 00",
            "Transfer-Encoding: chunked",
            "Expect: 100-continue",
        ):
            with self.subTest(extra=extra), self.assertRaises(proxy.Refused) as caught:
                proxy.check_bodiless(self.HEAD + "\r\n" + extra, b"")
            self.assertEqual(caught.exception.status, 400)


class PlainHttpServe(unittest.TestCase):
    def setUp(self):
        self.vm, self.conn = socket.socketpair()
        self.addCleanup(self.vm.close)
        self.addCleanup(self.conn.close)
        handle, self.logfile = tempfile.mkstemp()
        os.close(handle)
        self.addCleanup(os.unlink, self.logfile)

    def test_body_is_refused_before_any_lookup_or_connection(self):
        refuse = mock.patch.object(proxy, "connect_upstream", side_effect=AssertionError("connected upstream"))
        with mock.patch.object(proxy.socket, "getaddrinfo") as lookup, refuse as connect:
            self.vm.sendall(b"GET http://static.rust-lang.org/ HTTP/1.1\r\nContent-Length: 4\r\n\r\nabcd")
            proxy.serve(self.conn, ALLOW, self.logfile)
        self.assertTrue(receive(self.vm, 12).startswith(b"HTTP/1.1 400"))
        lookup.assert_not_called()
        connect.assert_not_called()

    def test_serve_sends_one_request_and_relays_only_the_answer(self):
        upstream_side, upstream = socket.socketpair()
        self.addCleanup(upstream_side.close)
        self.vm.sendall(b"GET http://static.rust-lang.org/crl HTTP/1.1\r\nAccept: */*\r\n\r\n")
        with resolving_to("93.184.216.34"), mock.patch.object(proxy, "connect_upstream", return_value=upstream):
            worker = threading.Thread(target=proxy.serve, args=(self.conn, ALLOW, self.logfile), daemon=True)
            worker.start()
            sent = receive_until(upstream_side, b"\r\n\r\n")
            self.vm.sendall(b"GET / HTTP/1.1\r\nHost: unlisted.example\r\n\r\n")
            upstream_side.sendall(b"HTTP/1.1 200 OK\r\n\r\nok")
            upstream_side.shutdown(socket.SHUT_WR)
            self.assertEqual(receive(self.vm, 21), b"HTTP/1.1 200 OK\r\n\r\nok")
            worker.join(timeout=10)
        self.assertFalse(worker.is_alive(), "relay did not end when the upstream closed")
        self.assertEqual(sent, b"GET /crl HTTP/1.1\r\nHost: static.rust-lang.org\r\nAccept: */*\r\nConnection: close\r\n\r\n")
        upstream_side.settimeout(1)
        self.assertNotIn(b"unlisted", upstream_side.recv(4096))


class LogSize(unittest.TestCase):
    def test_vm_text_in_a_log_line_is_cut_short(self):
        handle, logfile = tempfile.mkstemp()
        os.close(handle)
        self.addCleanup(os.unlink, logfile)
        proxy.log(logfile, "refused 403: not a host name: " + "a" * 60000)
        with open(logfile, encoding="utf-8") as read:
            self.assertLess(len(read.read()), proxy.MAX_LOGGED_CHARS + 50)

    def test_long_request_line_is_refused_before_it_is_parsed(self):
        with self.assertRaises(proxy.Refused) as caught:
            proxy.checked_head("CONNECT " + "a" * proxy.MAX_REQUEST_LINE + ":443 HTTP/1.1")
        self.assertEqual(caught.exception.status, 414)


class Deadlines(unittest.TestCase):
    """Every wait in the proxy ends: a silent VM or upstream never holds it open.

    Each test shortens one deadline to 0.2 s and allows 5 s, well under the
    real deadline it stands for."""

    BOUND = 5

    def pair(self):
        a, b = socket.socketpair()
        self.addCleanup(a.close)
        self.addCleanup(b.close)
        return a, b

    def bounded(self, call):
        """Runs `call` in a thread and fails unless it ends within BOUND
        seconds; re-raises what it raised."""
        outcome = {}

        def run():
            try:
                call()
            except BaseException as err:  # handed back to the test below
                outcome["error"] = err

        worker = threading.Thread(target=run, daemon=True)
        worker.start()
        worker.join(self.BOUND)
        self.assertFalse(worker.is_alive(), f"still waiting after {self.BOUND} s")
        if "error" in outcome:
            raise outcome["error"]

    def test_header_that_never_ends_times_out(self):
        vm, conn = self.pair()
        vm.sendall(b"CONNECT static.rust-lang.org:443 HTTP/1.1\r\n")
        with mock.patch.object(proxy, "HEADER_TIMEOUT", 0.2), self.assertRaises(TimeoutError):
            self.bounded(lambda: proxy.read_header(conn))

    def test_header_trickled_byte_by_byte_still_hits_one_deadline(self):
        vm, conn = self.pair()
        stop = threading.Event()

        def trickle():
            while not stop.wait(0.05):
                try:
                    vm.sendall(b"x")
                except OSError:
                    return

        threading.Thread(target=trickle, daemon=True).start()
        self.addCleanup(stop.set)
        with mock.patch.object(proxy, "HEADER_TIMEOUT", 0.5), self.assertRaises(TimeoutError):
            self.bounded(lambda: proxy.read_header(conn))

    def test_relay_ends_when_both_sides_stay_silent(self):
        _, client = self.pair()
        _, upstream = self.pair()
        with mock.patch.object(proxy, "IDLE_TIMEOUT", 0.2):
            self.bounded(lambda: proxy.relay(client, upstream))

    def test_send_to_a_peer_that_never_reads_times_out(self):
        vm, client = self.pair()
        _, upstream = self.pair()
        upstream.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 4096)
        filler = threading.Thread(target=self.flood, args=(vm,), daemon=True)
        filler.start()
        with mock.patch.object(proxy, "IDLE_TIMEOUT", 0.2), self.assertRaises(TimeoutError):
            self.bounded(lambda: proxy.relay(client, upstream))

    @staticmethod
    def flood(sock):
        try:
            while True:
                sock.sendall(b"x" * 65536)
        except OSError:
            pass

    def test_stalled_lookup_is_refused_in_time(self):
        def stall(*_args, **_kwargs):
            time.sleep(3)
            return PUBLIC

        with mock.patch.object(proxy, "DNS_TIMEOUT", 0.2), mock.patch.object(proxy.socket, "getaddrinfo", stall):
            with self.assertRaises(proxy.Refused) as caught:
                self.bounded(lambda: proxy.check_target("CONNECT", "static.rust-lang.org", 443, ALLOW))
        self.assertEqual(caught.exception.status, 504)

    def test_next_address_is_tried_when_a_socket_cannot_be_made(self):
        sock = mock.MagicMock()
        with mock.patch.object(proxy.socket, "socket", side_effect=[OSError("IPv6 is off"), sock]):
            found = proxy.connect_upstream([(socket.AF_INET6, ("2001:db8::1", 443, 0, 0)), (socket.AF_INET, ("93.184.216.34", 443))])
        self.assertIs(found, sock)
        sock.connect.assert_called_once_with(("93.184.216.34", 443))

    def test_failed_connect_closes_its_socket(self):
        sock = mock.MagicMock()
        sock.connect.side_effect = OSError("refused")
        with mock.patch.object(proxy.socket, "socket", return_value=sock), self.assertRaises(proxy.Refused):
            proxy.connect_upstream([(socket.AF_INET, ("93.184.216.34", 443))])
        sock.close.assert_called_once_with()

    def test_upstream_connect_has_a_deadline(self):
        sock = mock.MagicMock()
        with mock.patch.object(proxy.socket, "socket", return_value=sock):
            proxy.connect_upstream([(socket.AF_INET, ("93.184.216.34", 443))])
        self.assertEqual(sock.method_calls[:2], [mock.call.settimeout(proxy.CONNECT_TIMEOUT), mock.call.connect(("93.184.216.34", 443))])


class LoadAllowlist(unittest.TestCase):
    def test_comments_blank_lines_and_case(self):
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as handle:
            handle.write("# note\n\nStatic.Rust-Lang.org  # trailing\n.crates.io\n")
        self.addCleanup(os.unlink, handle.name)
        self.assertEqual(proxy.load_allowlist([handle.name]), {"static.rust-lang.org", ".crates.io"})

    def test_shipped_lists_parse_and_list_no_address_literal(self):
        here = os.path.dirname(os.path.abspath(__file__))
        paths = [os.path.join(here, name) for name in ("allow-run.txt", "allow-build.txt")]
        entries = proxy.load_allowlist(paths)
        self.assertIn("static.rust-lang.org", entries)
        for entry in entries:
            self.assertFalse(proxy.is_ip_literal(entry.lstrip(".")), entry)


if __name__ == "__main__":
    unittest.main()
