#!/usr/bin/env bash
# The critique loop's three sheets (motion-course.md step 11), from any render:
#   ./critique.sh out/animatic.mp4 [strip_start_s] [tag]
# -> out/critique/<tag>-contact.png  2 fps, 6 across (whole film)
#    out/critique/<tag>-strip.png    12 consecutive frames from strip_start_s
#    out/critique/<tag>-phone.png    1 fps at 360 px wide (phone readability)
set -euo pipefail
cd "$(dirname "$0")"
IN="$1"; SS="${2:-36.2}"; TAG="${3:-$(basename "${IN%.*}")}"
mkdir -p out/critique
DUR=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$IN")
ROWS=$(python3 -c "import math;print(math.ceil(float('$DUR')*2/6))")
PROWS=$(python3 -c "import math;print(math.ceil(float('$DUR')/8))")
W=$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 "$IN")
H=$(ffprobe -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 "$IN")
if [ "$H" -gt "$W" ]; then CW=180; else CW=270; fi
ffmpeg -loglevel error -y -i "$IN" -vf "fps=2,scale=${CW}:-1,tile=6x${ROWS}:padding=4:color=gray" -frames:v 1 "out/critique/$TAG-contact.png"
ffmpeg -loglevel error -y -ss "$SS" -i "$IN" -vf "scale=320:-1,tile=12x1:padding=2" -frames:v 1 "out/critique/$TAG-strip.png"
ffmpeg -loglevel error -y -i "$IN" -vf "fps=1,scale=360:-1,tile=8x${PROWS}:padding=6:color=gray" -frames:v 1 "out/critique/$TAG-phone.png"
ls -la out/critique/$TAG-*
