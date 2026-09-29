#!/bin/sh
# audio/master.sh — two-pass ffmpeg loudnorm mastering pass for the
# synthesized score: out/score.wav -> out/score-master.wav.
# Target: I=-14 LUFS, TP=-1.0 dBTP, LRA=11 (per the film's audio spec).
#
# Usage: sh audio/master.sh [in.wav] [out.wav]

set -eu

IN="${1:-out/score.wav}"
OUT="${2:-out/score-master.wav}"
I=-14
TP=-1.0
LRA=11

if [ ! -f "$IN" ]; then
  echo "master.sh: input not found: $IN" >&2
  exit 1
fi

echo "== master.sh: pass 1 (measure loudness of $IN) =="
MEASURE=$(ffmpeg -hide_banner -nostats -i "$IN" -af "loudnorm=I=$I:TP=$TP:LRA=$LRA:print_format=json" -f null - 2>&1)

# ffmpeg prints one JSON object at the end of stderr for pass 1.
JSON=$(printf '%s\n' "$MEASURE" | awk '/^\{/{f=1} f{print} /^\}/{if(f)exit}')
if [ -z "$JSON" ]; then
  echo "master.sh: could not parse loudnorm pass-1 output:" >&2
  printf '%s\n' "$MEASURE" >&2
  exit 1
fi

get() { printf '%s\n' "$JSON" | grep -o "\"$1\" *: *\"[^\"]*\"" | head -1 | sed -E 's/.*: *"([^"]*)"/\1/'; }

MEASURED_I=$(get input_i)
MEASURED_TP=$(get input_tp)
MEASURED_LRA=$(get input_lra)
MEASURED_THRESH=$(get input_thresh)
OFFSET=$(get target_offset)

echo "measured: I=$MEASURED_I LUFS  TP=$MEASURED_TP dBTP  LRA=$MEASURED_LRA  thresh=$MEASURED_THRESH  offset=$OFFSET"

echo "== master.sh: pass 2 (apply, linear normalization) =="
ffmpeg -hide_banner -nostats -y -i "$IN" -af \
  "loudnorm=I=$I:TP=$TP:LRA=$LRA:measured_I=$MEASURED_I:measured_TP=$MEASURED_TP:measured_LRA=$MEASURED_LRA:measured_thresh=$MEASURED_THRESH:offset=$OFFSET:linear=true:print_format=summary" \
  -ar 48000 -c:a pcm_s24le "$OUT"

echo "== master.sh: verifying result ($OUT) =="
RESULT=$(ffmpeg -hide_banner -nostats -i "$OUT" -af "loudnorm=I=$I:TP=$TP:LRA=$LRA:print_format=json" -f null - 2>&1)
RJSON=$(printf '%s\n' "$RESULT" | awk '/^\{/{f=1} f{print} /^\}/{if(f)exit}')
FINAL_I=$(printf '%s\n' "$RJSON" | grep -o '"input_i" *: *"[^"]*"' | head -1 | sed -E 's/.*: *"([^"]*)"/\1/')
FINAL_TP=$(printf '%s\n' "$RJSON" | grep -o '"input_tp" *: *"[^"]*"' | head -1 | sed -E 's/.*: *"([^"]*)"/\1/')

echo "$OUT measured integrated loudness: ${FINAL_I} LUFS (target ${I}), true peak: ${FINAL_TP} dBTP (target ${TP})"
