"""限速 / 抖动 HTTP 服务：模拟网络抖动与慢速链路。

用法：
    python slow_server.py <port> <file> [min_delay_ms] [max_delay_ms] [stall_prob]

- 每 64KB 发一块，块间随机 sleep [min, max] 毫秒
- 以 stall_prob 的概率插入一次 500ms 的停顿（模拟丢包重传 / 拥塞）
仅用于本地测试。
"""

import random
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
FILE = sys.argv[2]
MIN_MS = int(sys.argv[3]) if len(sys.argv) > 3 else 5
MAX_MS = int(sys.argv[4]) if len(sys.argv) > 4 else 30
STALL = float(sys.argv[5]) if len(sys.argv) > 5 else 0.03

DATA = open(FILE, "rb").read()
CHUNK = 64 * 1024


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "audio/mpeg")
        self.send_header("Content-Length", str(len(DATA)))
        self.end_headers()
        served = 0
        while served < len(DATA):
            end = min(served + CHUNK, len(DATA))
            try:
                self.wfile.write(DATA[served:end])
                self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                return
            served = end
            delay = random.uniform(MIN_MS, MAX_MS) / 1000.0
            if random.random() < STALL:
                delay += 0.5
            time.sleep(delay)

    def log_message(self, *_args):
        pass


if __name__ == "__main__":
    print(f"[slow_server] {PORT} file={FILE} delay={MIN_MS}-{MAX_MS}ms stall={STALL}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
