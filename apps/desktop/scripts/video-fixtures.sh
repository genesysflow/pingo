#!/bin/sh
set -eu
fixtures="${1:-/tmp/pingo-video-fixtures}"
mkdir -p "$fixtures"
for fps in 30 60; do
  ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc2=size=1920x1080:rate=$fps" -t 5 -c:v libx264 -threads 2 -preset ultrafast -tune zerolatency -profile:v baseline -x264-params "aud=1:repeat-headers=1:keyint=$fps" -f h264 "$fixtures/$fps.h264"
  ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc2=size=1920x1080:rate=$fps" -t 5 -c:v libvpx -threads 2 -deadline realtime -cpu-used 8 -b:v 4M -g "$fps" -f ivf "$fixtures/$fps.ivf"
done
python3 - "$fixtures" <<'PY'
from pathlib import Path
import sys
p=Path(sys.argv[1]); b=p.joinpath('30.h264').read_bytes(); n=b.find(b'\0\0\0\x01\x09',6)
p.joinpath('first.h264').write_bytes(b[:n]); b=p.joinpath('30.ivf').read_bytes(); n=int.from_bytes(b[32:36],'little')
p.joinpath('first.vp8').write_bytes(b[44:44+n])
PY
