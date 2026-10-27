# montty icon

Source: the author's original drawing of Monty (2048x2048 RGBA), which is not in the repo. It is never modified.
The icon is the 1680x1680 square at (125, 150), scaled to 2048 with Lanczos, original alpha kept
(about a 1.22x crop). The art is not redrawn or recoloured. Sizes: 16, 24, 32, 48, 64, 128, 256, 512.

Run from the repo root:

```bash
SRC=path/to/original.png
T=$(mktemp -d)
ffmpeg -loglevel error -y -i "$SRC" -filter_complex \
  "[0]crop=1680:1680:125:150,scale=2048:2048:flags=lanczos,format=rgba[z];[0]alphaextract[a];[z][a]alphamerge" \
  "$T/crop.png"
for n in 16 24 32 48 64 128 256 512; do
  mkdir -p assets/icon/hicolor/${n}x${n}/apps
  ffmpeg -loglevel error -y -i "$T/crop.png" -vf scale=$n:$n:flags=lanczos -pix_fmt rgba \
    assets/icon/hicolor/${n}x${n}/apps/montty.png
done
```
