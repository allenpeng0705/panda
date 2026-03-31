#!/usr/bin/env bash
set -euo pipefail

# Soak guard for SSE traffic and process resource creep.
# Example:
#   PANDA_BASE_URL=http://127.0.0.1:8080 \
#   SOAK_PAYLOAD=./payload_stream.json \
#   SOAK_DURATION_SECONDS=3600 \
#   SOAK_CONCURRENCY=10 \
#   SOAK_PID=$(pgrep -f "/usr/local/bin/panda|target/release/panda" | head -n1) \
#   ./scripts/soak_guard_sse.sh

BASE_URL="${PANDA_BASE_URL:-http://127.0.0.1:8080}"
PAYLOAD="${SOAK_PAYLOAD:-}"
DURATION="${SOAK_DURATION_SECONDS:-3600}"
CONC="${SOAK_CONCURRENCY:-10}"
PID="${SOAK_PID:-}"
AUTH_HEADER="${SOAK_AUTH_HEADER:-}"
OUT_DIR="${SOAK_OUTPUT_DIR:-artifacts/soak}"
SAMPLES_FILE="${OUT_DIR}/soak_samples_$(date +%Y%m%d_%H%M%S).csv"

if [[ -z "${PAYLOAD}" || ! -f "${PAYLOAD}" ]]; then
  echo "SOAK_PAYLOAD must point to an existing JSON file (stream=true request)"
  exit 1
fi
if [[ -z "${PID}" ]]; then
  echo "SOAK_PID is required (target panda process id)"
  exit 1
fi

mkdir -p "${OUT_DIR}"
echo "epoch_s,rss_kb,open_fds,active_tcp" > "${SAMPLES_FILE}"

end_ts=$(( $(date +%s) + DURATION ))
echo "base_url=${BASE_URL} duration_s=${DURATION} concurrency=${CONC} pid=${PID} samples=${SAMPLES_FILE}"

monitor_once() {
  local ts rss fds tcp
  ts="$(date +%s)"
  rss="$(ps -o rss= -p "${PID}" | awk '{print $1}')"
  fds="$(ls "/proc/${PID}/fd" 2>/dev/null | wc -l | awk '{print $1}')"
  if [[ "$(uname)" == "Darwin" ]]; then
    fds="$(lsof -p "${PID}" 2>/dev/null | wc -l | awk '{print $1}')"
    tcp="$(lsof -nP -iTCP -a -p "${PID}" 2>/dev/null | wc -l | awk '{print $1}')"
  else
    tcp="$(ss -tanp 2>/dev/null | rg -c "${PID}" || true)"
  fi
  echo "${ts},${rss:-0},${fds:-0},${tcp:-0}" >> "${SAMPLES_FILE}"
}

while [[ "$(date +%s)" -lt "${end_ts}" ]]; do
  monitor_once
  if [[ -n "${AUTH_HEADER}" ]]; then
    seq "${CONC}" | xargs -I{} -P "${CONC}" curl -sS -N -H "Content-Type: application/json" -H "${AUTH_HEADER}" --data-binary @"${PAYLOAD}" "${BASE_URL}/v1/chat/completions" >/dev/null || true
  else
    seq "${CONC}" | xargs -I{} -P "${CONC}" curl -sS -N -H "Content-Type: application/json" --data-binary @"${PAYLOAD}" "${BASE_URL}/v1/chat/completions" >/dev/null || true
  fi
done

python3 - <<PY
import csv
from pathlib import Path
p = Path("${SAMPLES_FILE}")
rows = list(csv.DictReader(p.open()))
if len(rows) < 2:
    print("insufficient soak samples")
    raise SystemExit(1)
first, last = rows[0], rows[-1]
def iv(x): 
    try: return int(x)
    except: return 0
rss_delta = iv(last["rss_kb"]) - iv(first["rss_kb"])
fds_delta = iv(last["open_fds"]) - iv(first["open_fds"])
tcp_delta = iv(last["active_tcp"]) - iv(first["active_tcp"])
print(f"samples={len(rows)} rss_delta_kb={rss_delta} fds_delta={fds_delta} tcp_delta={tcp_delta}")
PY
