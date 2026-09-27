"""Run a command, kill it if RSS > LIMIT_MB or wall > LIMIT_S. Reports child ru_maxrss.
Usage: python guard.py LIMIT_MB LIMIT_S cmd...
"""
import sys, os, subprocess, time, signal
limit_mb, limit_s = float(sys.argv[1]), float(sys.argv[2])
p = subprocess.Popen(sys.argv[3:])
t0 = time.time(); maxpoll = 0; reason = None
while True:
    pid, status, ru = os.wait4(p.pid, os.WNOHANG)
    if pid:
        break
    try:
        rss = int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(p.pid)]).strip() or 0) / 1024
    except Exception:
        rss = 0
    maxpoll = max(maxpoll, rss)
    el = time.time() - t0
    if rss > limit_mb or el > limit_s:
        reason = f"KILLED rss={rss:.0f}MB elapsed={el:.0f}s"
        p.send_signal(signal.SIGKILL)
        pid, status, ru = os.wait4(p.pid, 0)
        break
    time.sleep(1)
print(f"[guard] wall={time.time()-t0:.1f}s ru_maxrss={ru.ru_maxrss/2**20:.0f}MB polled_max={maxpoll:.0f}MB "
      f"exit={status} {reason or ''}", file=sys.stderr)
