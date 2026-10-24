"""CPU 压力：起 N 个忙循环进程，持续 T 秒。

用法：
    python cpu_stress.py [秒数] [worker 数]

作用等同 Linux 的 `stress-ng --cpu N --timeout T`（本机是 Windows，没有 stress-ng）。
每个 worker 做整数乘加循环，吃满一个核。
"""

import multiprocessing
import sys
import time


def burn(_):
    x = 12345
    while True:
        x = (x * 1103515245 + 12345) & 0x7FFFFFFF
        x ^= x >> 7


if __name__ == "__main__":
    secs = int(sys.argv[1]) if len(sys.argv) > 1 else 60
    workers = int(sys.argv[2]) if len(sys.argv) > 2 else multiprocessing.cpu_count()
    procs = [multiprocessing.Process(target=burn, args=(i,), daemon=True) for i in range(workers)]
    for p in procs:
        p.start()
    print(f"[stress] {workers} workers 跑 {secs}s", flush=True)
    time.sleep(secs)
    for p in procs:
        p.terminate()
    print("[stress] 结束", flush=True)
