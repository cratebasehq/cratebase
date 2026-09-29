#!/usr/bin/env bash
# Deliverables from the near-lossless masters (render.mjs) + mastered score.
#   ./encode.sh [all|mp4|webm|teaser|stills]   # everything, or one step
#   ./encode.sh mux <video> <out.mp4>   # quick mux for critique renders
# Needs: out/master-16x9.mp4, out/master-9x16.mp4 (./render.sh master ...),
#        out/score-master.wav (node audio/score.mjs && sh audio/master.sh).
set -euo pipefail
cd "$(dirname "$0")"
A=out/score-master.wav
# picture is exactly 64.000 s; the score's tail is faded under the last 1.5 s
AF="atrim=0:64,afade=t=out:st=62.5:d=1.5"

mux() { ffmpeg -loglevel error -y -i "$1" -i "$A" -filter_complex "[1:a]$AF[a]" -map 0:v -map "[a]" -c:v copy -c:a aac -b:a 192k -shortest -movflags +faststart "$2"; }

h264() { # in out maxrate(k) — two-pass-free size control via capped CRF
  ffmpeg -loglevel error -y -i "$1" -i "$A" -filter_complex "[1:a]$AF[a]" -map 0:v -map "[a]" \
    -c:v libx264 -preset slow -crf "${4:-20}" -maxrate "$3k" -bufsize "$(( $3 * 2 ))k" -pix_fmt yuv420p -profile:v high -tune animation \
    -c:a aac -b:a 160k -movflags +faststart "$2"
}

if [ "${1:-}" = "mux" ]; then mux "$2" "$3"; exit; fi
STEP="${1:-all}"   # all | mp4 | webm | teaser | stills
mkdir -p out/delivery
if [ "$STEP" = all ] || [ "$STEP" = mp4 ]; then
h264 out/master-16x9.mp4 out/delivery/cratebase-launch-16x9.mp4 2700 18
h264 out/master-9x16.mp4 out/delivery/cratebase-launch-9x16.mp4 2700 18
fi
if [ "$STEP" = all ] || [ "$STEP" = webm ]; then
# VP9 WebM (two-pass, constrained quality)
ffmpeg -loglevel error -y -i out/master-16x9.mp4 -c:v libvpx-vp9 -b:v 2000k -crf 30 -pass 1 -row-mt 1 -an -f null /dev/null
ffmpeg -loglevel error -y -i out/master-16x9.mp4 -i "$A" -filter_complex "[1:a]$AF[a]" -map 0:v -map "[a]" \
  -c:v libvpx-vp9 -b:v 2000k -crf 30 -pass 2 -row-mt 1 -c:a libopus -b:a 160k out/delivery/cratebase-launch-16x9.webm
rm -f ffmpeg2pass-0.log
fi
if [ "$STEP" = all ] || [ "$STEP" = teaser ]; then
# 15 s teaser: master segments on beat boundaries (docs/shotlist.md "Teaser"),
# audio cut at the same points with 12 ms crossfades
SEG="0:4 36:40 22.5:25 48:50 56:58.5"
i=0; V=""; AU=""; F=""
for s in $SEG; do a=${s%:*}; b=${s#*:}
  F="$F[0:v]trim=$a:$b,setpts=PTS-STARTPTS[v$i];[1:a]atrim=$a:$b,asetpts=PTS-STARTPTS,afade=t=in:d=0.012,afade=t=out:st=$(python3 -c "print($b-$a-0.012)"):d=0.012[a$i];"
  V="$V[v$i][a$i]"; i=$((i+1)); done
ffmpeg -loglevel error -y -i out/master-16x9.mp4 -i "$A" -filter_complex "${F}${V}concat=n=$i:v=1:a=1[v][a];[a]afade=t=out:st=14.2:d=0.8[a2];[v]fps=60[v2]" \
  -map "[v2]" -map "[a2]" -r 60 -t 15 -c:v libx264 -preset slow -crf 19 -maxrate 4000k -bufsize 8000k -pix_fmt yuv420p -c:a aac -b:a 160k -movflags +faststart out/delivery/cratebase-teaser-15s-16x9.mp4
fi
if [ "$STEP" = all ] || [ "$STEP" = stills ]; then
# poster = clean last frame; key stills
ffmpeg -loglevel error -y -sseof -0.02 -i out/master-16x9.mp4 -frames:v 1 -update 1 out/delivery/poster-16x9.png
ffmpeg -loglevel error -y -sseof -0.02 -i out/master-9x16.mp4 -frames:v 1 -update 1 out/delivery/poster-9x16.png
n=1; for t in 1.6 4.3 17.3 27.0 42.9 49.5; do
  ffmpeg -loglevel error -y -ss "$t" -i out/master-16x9.mp4 -frames:v 1 "out/delivery/still-$n-${t}s.png"; n=$((n+1)); done
./critique.sh out/delivery/cratebase-launch-16x9.mp4 36.2 final
cp out/critique/final-contact.png out/delivery/contact-sheet-16x9.png
fi
ls -la out/delivery
for f in out/delivery/*.mp4 out/delivery/*.webm; do
  printf "%s  " "$f"; ffprobe -v error -show_entries format=duration,size:stream=codec_name,width,height,r_frame_rate,pix_fmt -of compact=p=0:nk=1 "$f" | tr '\n' ' '; echo; done
