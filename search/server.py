#!/usr/bin/env python3
"""Obscura Metasearch — AI-powered metasearch on :8000.

Queries DDG + Bing + Wikipedia via the Rust metasearch binary,
then synthesizes a cited answer using the glm-5.2 LLM.
"""
import http.server
import json
import os
import subprocess
import urllib.parse
import urllib.request

PORT = 8000
BASE_DIR = os.path.dirname(os.path.abspath(__file__))
OBSCURA_DIR = os.path.expanduser("~/obscura")
METASEARCH_BIN = os.path.join(OBSCURA_DIR, "target", "release", "examples", "metasearch_cli")
INDEX_HTML = os.path.join(BASE_DIR, "index.html")

# LLM config — glm-5.2 via FuturePPO API (OpenAI-compatible)
LLM_API_KEY = "sk-FyMXjeQF2hMH0VyGWATqxpthVzc2kDG5B5ttGdSgwij7CmAH"
LLM_BASE_URL = "https://api.futureppo.top/v1"
LLM_MODEL = "glm-5.2"


class SearchHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/":
            self.serve_file(INDEX_HTML, "text/html")
        elif parsed.path == "/api/search":
            self.handle_search(parsed.query)
        else:
            self.send_error(404)

    def do_POST(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/api/synthesize":
            self.handle_synthesize()
        elif parsed.path == "/api/search_and_synthesize":
            self.handle_search_and_synthesize(parsed.query)
        else:
            self.send_error(404)

    def serve_file(self, path, content_type):
        try:
            with open(path, "rb") as f:
                content = f.read()
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(content)))
            self.end_headers()
            self.wfile.write(content)
        except FileNotFoundError:
            self.send_error(404, "File not found")

    def run_metasearch(self, query):
        """Run the Rust metasearch binary and return parsed results."""
        result = subprocess.run(
            [METASEARCH_BIN, query],
            capture_output=True,
            text=True,
            timeout=45,
        )
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip() or "Search failed")
        return json.loads(result.stdout.strip())

    def synthesize_answer(self, query, results):
        """Call the glm-5.2 LLM to synthesize a cited answer from search results."""
        context_parts = []
        for i, r in enumerate(results[:15], 1):
            context_parts.append(
                f"[{i}] {r.get('title', '')}\n"
                f"    URL: {r.get('url', '')}\n"
                f"    Snippet: {r.get('snippet', '')}\n"
                f"    Source: {r.get('source', '')}"
            )
        context = "\n\n".join(context_parts)

        system_prompt = (
            "You are an AI research assistant. Based on the search results provided, "
            "write a comprehensive, well-structured answer to the user's question. "
            "Use inline citation numbers [1], [2], etc. that correspond to the numbered sources. "
            "Be thorough, accurate, and objective. Highlight key facts, dates, and figures when present. "
            "If sources conflict, note the disagreement. Keep the answer concise but informative."
        )

        user_prompt = (
            f"Question: {query}\n\n"
            f"Search Results:\n{context}\n\n"
            "Provide a clear, well-structured answer with inline citations "
            "[1], [2], etc. referencing the sources above."
        )

        payload = json.dumps({
            "model": LLM_MODEL,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ],
            "temperature": 0.3,
            "max_tokens": 2000,
        }).encode("utf-8")

        req = urllib.request.Request(
            f"{LLM_BASE_URL}/chat/completions",
            data=payload,
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {LLM_API_KEY}",
            },
        )

        with urllib.request.urlopen(req, timeout=60) as resp:
            llm_response = json.loads(resp.read().decode("utf-8"))
            return llm_response.get("choices", [{}])[0].get("message", {}).get("content", "")

    def handle_search(self, query_string):
        """GET /api/search?q= — return metasearch results only."""
        params = urllib.parse.parse_qs(query_string)
        query = params.get("q", [""])[0].strip()

        if not query:
            self.send_json({"error": "No query provided"})
            return

        try:
            results = self.run_metasearch(query)
            self.send_json({
                "query": query,
                "results": results,
                "count": len(results),
                "providers": ["duckduckgo", "bing", "wikipedia"],
            })
        except subprocess.TimeoutExpired:
            self.send_json({"error": "Search timed out (45s)"})
        except FileNotFoundError:
            self.send_json({"error": "Metasearch binary not built. Run: cargo build --release -p obscura-js --example metasearch_cli"})
        except Exception as e:
            self.send_json({"error": str(e)})

    def handle_synthesize(self):
        """POST /api/synthesize — body: {query, results}. Returns AI-synthesized answer."""
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length > 0 else b"{}"

        try:
            data = json.loads(body)
        except json.JSONDecodeError:
            self.send_json({"error": "Invalid JSON body"})
            return

        query = data.get("query", "")
        results = data.get("results", [])

        if not query or not results:
            self.send_json({"error": "Missing query or results"})
            return

        try:
            answer = self.synthesize_answer(query, results)
            self.send_json({"answer": answer, "query": query})
        except Exception as e:
            self.send_json({"error": f"LLM synthesis failed: {e}"})

    def handle_search_and_synthesize(self, query_string):
        """POST /api/search_and_synthesize?q= — run metasearch + synthesize in one call."""
        params = urllib.parse.parse_qs(query_string)
        query = params.get("q", [""])[0].strip()

        if not query:
            self.send_json({"error": "No query provided"})
            return

        try:
            results = self.run_metasearch(query)
        except Exception as e:
            self.send_json({"error": str(e)})
            return

        try:
            answer = self.synthesize_answer(query, results)
        except Exception as e:
            # Return results even if synthesis fails
            self.send_json({
                "query": query,
                "results": results,
                "count": len(results),
                "providers": ["duckduckgo", "bing", "wikipedia"],
                "error": f"LLM synthesis failed: {e}",
            })
            return

        self.send_json({
            "query": query,
            "answer": answer,
            "results": results,
            "count": len(results),
            "providers": ["duckduckgo", "bing", "wikipedia"],
        })

    def send_json(self, data):
        body = json.dumps(data).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()
        self.wfile.write(body)

    def do_OPTIONS(self):
        self.send_response(200)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()

    def log_message(self, format, *args):
        print(f"[{self.log_date_time_string()}] {args[0]}")


def main():
    server = http.server.HTTPServer(("0.0.0.0", PORT), SearchHandler)
    print("╔══════════════════════════════════════════════╗")
    print(f"║  Obscura Metasearch — http://localhost:{PORT}    ║")
    print("║  Providers: DDG + Bing + Wikipedia            ║")
    print("║  AI Synthesis: glm-5.2                        ║")
    print("╚══════════════════════════════════════════════╝")
    print()
    server.serve_forever()


if __name__ == "__main__":
    main()
