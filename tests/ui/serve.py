"""Local UI fixture; synthetic IPC only. Run npm run build first."""
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler
from pathlib import Path

root = Path(__file__).resolve().parent
dist = root.parents[1] / "dist"


class Handler(SimpleHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?")[0]
        if path in ("/", "/app.html"):
            data = (dist / "index.html").read_text(encoding="utf-8").replace(
                "<head>", '<head><script src="/mock.js"></script>'
            ).encode("utf-8")
        elif path == "/mock.js":
            data = (root / "mock.js").read_bytes()
        elif path.startswith("/assets/") and ".." not in path:
            self.directory = str(dist)
            return super().do_GET()
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/javascript; charset=utf-8" if path.endswith(".js") else "text/html; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


print("Synthetic UI fixture: http://127.0.0.1:18743", flush=True)
ThreadingHTTPServer(("127.0.0.1", 18743), Handler).serve_forever()
