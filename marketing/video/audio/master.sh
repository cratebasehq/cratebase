#!/bin/sh
# audio/master.sh — master the synthesized score to -14 LUFS / -1 dBTP.
#   sh audio/master.sh [in.wav] [out.wav]
# Linear only (no dynamic loudnorm pumping): measure integrated loudness,
# apply one static gain to land on -14 LUFS, then a 4x-oversampled
# look-ahead limiter catches the few transient peaks (the two hits) so
# true peak stays under -1 dBTP. Prints the verified result.
set -eu
IN="${1:-out/score.wav}"
OUT="${2:-out/score-master.wav}"
measure() { ffmpeg -hide_banner -nostats -i "$1" -af "loudnorm=I=-14:TP=-1:LRA=11:print_format=json" -f null - 2>&1 | awk '/^\{/{f=1} f{print}' | grep -o "\"$2\" *: *\"[^\"]*\"" | head -1 | sed -E 's/.*: *"([^"]*)"/\1/'; }
I0=$(measure "$IN" input_i)
G=$(python3 -c "print(round(-14.0 - float('$I0') + 0.25, 2))")   # +0.25 dB pre-compensates limiter loss
echo "input: $I0 LUFS -> static gain ${G} dB"
ffmpeg -hide_banner -nostats -loglevel error -y -i "$IN" -af \
  "volume=${G}dB,aresample=192000,alimiter=limit=0.8:attack=2:release=80:level=false,aresample=48000" \
  -c:a pcm_s24le "$OUT"
echo "$OUT measured integrated loudness: $(measure "$OUT" input_i) LUFS (target -14), true peak: $(measure "$OUT" input_tp) dBTP (target -1.0)"
