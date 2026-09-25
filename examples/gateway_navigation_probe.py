"""Temporary loopback-only browser fixture. Prints metadata JSONL, no cookies.
Run python3 examples/gateway_navigation_probe.py PORT; port 0 picks a test port.
The /redirect path redirects to a different localhost origin, /page has links,
and /automatic creates a script navigation, fetch and iframe for negative cases.
No AppHost registry, daemon, project, or real service is accessed.
"""
import html
import http.server
import json
import sys


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        fields = {"host", "origin", "sec-fetch-dest", "sec-fetch-mode",
                  "sec-fetch-site", "sec-fetch-user", "sec-purpose", "purpose"}
        item = {"method": "GET", "path": self.path,
                "headers": [(k, v) for k, v in self.headers.items() if k.lower() in fields]}
        print(json.dumps(item), flush=True)
        target = "http://apphost-probe.localhost:" + str(self.server.server_port)
        if self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", target + "/redirect-target")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        body = ("<!doctype html><meta charset=utf-8><title>AppHost G2 temporary probe</title>"
                "<h1>AppHost G2 temporary probe</h1><pre>" +
                html.escape(json.dumps(item, indent=2)) + "</pre>" +
                '<a href="' + target + '/page-link">Navigate from page</a> ' +
                '<a href="/redirect">Redirect from page</a>')
        if self.path == "/automatic":
            body += ('<iframe src="' + target + '/frame"></iframe><script>' +
                     'fetch("' + target + '/fetch").catch(()=>{});' +
                     'setTimeout(()=>location.href="' + target + '/script-navigation",500);</script>')
        encoded = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_args):
        pass


if __name__ == "__main__":
    server = http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), Handler)
    print("PORT=" + str(server.server_port), file=sys.stderr, flush=True)
    server.serve_forever()
