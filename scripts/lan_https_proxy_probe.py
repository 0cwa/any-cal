#!/usr/bin/env python3
"""Disposable local-only Any-Cal listener/TLS proxy probe.

The probe prints only structural results. It uses synthetic credentials and a
temporary HOME; it does not inherit the caller's environment.
"""
import contextlib
import http.client
import os
import pathlib
import socket
import ssl
import subprocess
import tempfile
import threading
import time


ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = ROOT / "target/debug/any-cal-app"
TOKEN = "synthetic-lan-token-not-real"
LOCAL = "synthetic-local-health-not-real"


def free_port():
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    return port


def env(extra):
    result = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": tempfile.mkdtemp(prefix="any-cal-home-")}
    result.update(extra)
    return result


def wait_port(port, process):
    deadline = time.time() + 5
    while time.time() < deadline:
        if process.poll() is not None:
            return False
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.15):
                return True
        except OSError:
            time.sleep(0.04)
    return False


def fail_closed(extra):
    result = subprocess.run(
        [str(BIN), "check"],
        cwd=ROOT,
        env=env(extra),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        timeout=4,
    )
    return {
        "exit_nonzero": result.returncode != 0,
        "stderr_nonempty": bool(result.stderr),
        "stderr_has_synthetic_secret": TOKEN.encode() in result.stderr or LOCAL.encode() in result.stderr,
    }


def start_app(port, rate=120, local=False):
    settings = {
        "ANY_CAL_SPACE_ID": "synthetic-space",
        "ANY_CAL_AUTH_CREDENTIAL": TOKEN,
        "ANY_CAL_LISTEN_ADDRESS": f"127.0.0.1:{port}",
        "ANY_CAL_REVERSE_PROXY_TLS": "true",
        "ANY_CAL_RATE_LIMIT_PER_MINUTE": str(rate),
    }
    if local:
        settings["ANY_CAL_LOCAL_AUTH"] = LOCAL
    process = subprocess.Popen(
        [str(BIN), "serve"],
        cwd=ROOT,
        env=env(settings),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if not wait_port(port, process):
        process.terminate()
        process.wait(timeout=2)
        raise RuntimeError("listener did not start")
    return process


def request(port, path, auth=TOKEN, headers=None, body=b"", method="GET"):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
    values = {"Connection": "close"}
    if auth is not None:
        values["Authorization"] = f"Bearer {auth}"
    if headers:
        values.update(headers)
    conn.request(method, path, body=body, headers=values)
    response = conn.getresponse()
    status = response.status
    response.read()
    conn.close()
    return status


def request_location(port, path, headers):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
    values = {"Authorization": f"Bearer {TOKEN}", "Connection": "close"}
    values.update(headers)
    conn.request("GET", path, headers=values)
    response = conn.getresponse()
    status = response.status
    location = response.getheader("Location", "")
    response.read()
    conn.close()
    return status, location


class Proxy:
    def __init__(self, backend, cert, key):
        self.backend = backend
        self.cert = cert
        self.key = key
        self.port = free_port()
        self.stop = threading.Event()
        self.listener = None

    def run(self):
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(certfile=self.cert, keyfile=self.key)
        self.listener = socket.socket()
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(("127.0.0.1", self.port))
        self.listener.listen(8)
        self.listener.settimeout(0.2)
        while not self.stop.is_set():
            try:
                raw, _ = self.listener.accept()
            except socket.timeout:
                continue
            try:
                self.connection(raw, context)
            except (OSError, ssl.SSLError):
                with contextlib.suppress(OSError):
                    raw.close()

    def connection(self, raw, context):
        with context.wrap_socket(raw, server_side=True) as client:
            client.settimeout(3)
            with socket.create_connection(("127.0.0.1", self.backend), timeout=3) as backend:
                backend.settimeout(3)
                while True:
                    request_bytes = b""
                    while b"\r\n\r\n" not in request_bytes:
                        chunk = client.recv(4096)
                        if not chunk:
                            return
                        request_bytes += chunk
                        if len(request_bytes) > 65536:
                            return
                    marker = request_bytes.find(b"\r\n\r\n")
                    head, body = request_bytes[:marker], request_bytes[marker + 4 :]
                    if b"x-forwarded-proto:" not in head.lower():
                        head += b"\r\nX-Forwarded-Proto: https"
                    backend.sendall(head + b"\r\n\r\n" + body)
                    response = b""
                    while b"\r\n\r\n" not in response:
                        chunk = backend.recv(4096)
                        if not chunk:
                            return
                        response += chunk
                    marker = response.find(b"\r\n\r\n")
                    length = 0
                    for line in response[:marker].lower().split(b"\r\n"):
                        if line.startswith(b"content-length:"):
                            length = int(line.split(b":", 1)[1].strip())
                    while len(response) < marker + 4 + length:
                        response += backend.recv(4096)
                    client.sendall(response)
                    if b"connection: close" in response[:marker].lower():
                        return


def main():
    if not BIN.exists():
        raise SystemExit("missing target/debug/any-cal-app")
    backend = free_port()
    app = start_app(backend, local=True)
    proxy = None
    try:
        with tempfile.TemporaryDirectory(prefix="any-cal-tls-") as directory:
            directory = pathlib.Path(directory)
            cert = directory / "cert.pem"
            key = directory / "key.pem"
            subprocess.run(
                [
                    "openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                    "-keyout", str(key), "-out", str(cert), "-subj", "/CN=localhost",
                    "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1",
                ], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            proxy = Proxy(backend, cert, key)
            thread = threading.Thread(target=proxy.run, daemon=True)
            thread.start()
            deadline = time.time() + 3
            while proxy.listener is None and time.time() < deadline:
                time.sleep(0.02)

            result = {}
            result["config_missing_auth_fails_closed"] = fail_closed({
                "ANY_CAL_SPACE_ID": "synthetic-space",
                "ANY_CAL_LISTEN_ADDRESS": f"127.0.0.1:{free_port()}",
                "ANY_CAL_REVERSE_PROXY_TLS": "true",
            })
            result["config_nonloopback_fails_closed"] = fail_closed({
                "ANY_CAL_SPACE_ID": "synthetic-space",
                "ANY_CAL_LISTEN_ADDRESS": f"0.0.0.0:{free_port()}",
                "ANY_CAL_ALLOW_LAN": "true",
                "ANY_CAL_REVERSE_PROXY_TLS": "true",
                "ANY_CAL_AUTH_CREDENTIAL": TOKEN,
            })
            result["direct"] = {
                "no_auth": request(backend, "/health", auth=None),
                "wrong_auth": request(backend, "/health", auth="wrong-synthetic"),
                "health_auth": request(backend, "/health"),
                "local_health": request(backend, "/health", auth=LOCAL),
                "local_dav_denied": request(backend, "/carddav/contacts", auth=LOCAL),
                "global_dav": request(backend, "/carddav/contacts"),
                "other_space": request(backend, "/spaces/other-space/carddav/contacts"),
            }
            spoof_status, spoof_location = request_location(
                backend,
                "/.well-known/caldav",
                {"Host": "evil.example/redirect", "X-Forwarded-Proto": "https"},
            )
            result["untrusted_origin_relative"] = spoof_status == 302 and spoof_location == "/caldav/"

            trusted = ssl.create_default_context(cafile=str(cert))
            connection = http.client.HTTPSConnection("localhost", proxy.port, context=trusted, timeout=3)
            connection.request("GET", "/health", headers={"Authorization": f"Bearer {TOKEN}", "Connection": "close"})
            response = connection.getresponse(); result["tls_health"] = response.status; response.read(); connection.close()
            connection = http.client.HTTPSConnection("localhost", proxy.port, context=trusted, timeout=3)
            connection.request("GET", "/.well-known/caldav", headers={"Authorization": f"Bearer {TOKEN}", "Connection": "keep-alive"})
            response = connection.getresponse(); location = response.getheader("Location", ""); result["tls_discovery"] = [response.status, location == f"https://localhost:{proxy.port}/caldav/"]; response.read()
            connection.request("GET", "/health", headers={"Authorization": f"Bearer {TOKEN}", "Connection": "close"})
            response = connection.getresponse(); result["tls_keepalive_health"] = response.status; response.read(); connection.close()

            unknown = ssl.create_default_context()
            try:
                connection = http.client.HTTPSConnection("localhost", proxy.port, context=unknown, timeout=2)
                connection.request("GET", "/health")
                connection.getresponse()
                result["unknown_ca_rejected"] = False
            except (ssl.SSLError, OSError):
                result["unknown_ca_rejected"] = True

            try:
                raw = socket.create_connection(("127.0.0.1", proxy.port), timeout=2)
                wrong = ssl.create_default_context(cafile=str(cert))
                wrong.wrap_socket(raw, server_hostname="wronghost")
                result["wrong_hostname_rejected"] = False
            except (ssl.SSLError, OSError):
                result["wrong_hostname_rejected"] = True

            try:
                raw = socket.create_connection(("127.0.0.1", proxy.port), timeout=2)
                raw.sendall(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
                raw.settimeout(1)
                result["plaintext_downgrade_rejected"] = not raw.recv(64).startswith(b"HTTP/")
                raw.close()
            except (ssl.SSLError, OSError):
                result["plaintext_downgrade_rejected"] = True

            raw = socket.create_connection(("127.0.0.1", proxy.port), timeout=2)
            tls = trusted.wrap_socket(raw, server_hostname="localhost")
            tls.sendall((f"GET /health HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").encode())
            malformed = tls.recv(4096)
            tls.close()
            result["duplicate_length_400"] = b" 400 " in malformed.split(b"\r\n", 1)[0]
            result["malformed_response_redacted"] = TOKEN.encode() not in malformed

            rate_port = free_port()
            rate_app = start_app(rate_port, rate=2)
            try:
                result["rate_limit_statuses"] = [request(rate_port, "/health") for _ in range(3)]
            finally:
                rate_app.terminate(); rate_app.wait(timeout=3)

            print({
                "config_missing_auth_fails_closed": result["config_missing_auth_fails_closed"]["exit_nonzero"],
                "config_nonloopback_fails_closed": result["config_nonloopback_fails_closed"]["exit_nonzero"],
                "direct": result["direct"],
                "untrusted_origin_relative": result["untrusted_origin_relative"],
                "tls": {key: result[key] for key in ("tls_health", "tls_discovery", "tls_keepalive_health", "unknown_ca_rejected", "wrong_hostname_rejected", "plaintext_downgrade_rejected", "duplicate_length_400", "malformed_response_redacted")},
                "rate_limit_statuses": result["rate_limit_statuses"],
                "stderr_secret_free": not result["config_missing_auth_fails_closed"]["stderr_has_synthetic_secret"] and not result["config_nonloopback_fails_closed"]["stderr_has_synthetic_secret"],
            })
    finally:
        if proxy is not None:
            proxy.stop.set()
            if proxy.listener is not None:
                proxy.listener.close()
        app.terminate()
        app.wait(timeout=3)


if __name__ == "__main__":
    main()
