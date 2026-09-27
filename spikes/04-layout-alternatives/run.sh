#!/bin/sh
# usage: run.sh TAG METHOD [opts]  -> guarded (5 GB RSS, 30 min)
cd "$(dirname "$0")"
DYLD_FALLBACK_LIBRARY_PATH=/opt/homebrew/lib .venv/bin/python guard.py 5000 1800 .venv/bin/python bench.py "$@" 2>&1 | grep -v Warning | tail -3
