# Video upload E2E fixtures

These two synthetic, two-second, 640 × 360 H.264/yuv420p videos were generated
with FFmpeg's `testsrc2` source at 10 fps, without audio. The late-moov variant
remuxes the same stream without `+faststart`. They contain no user media.

`synthetic-vp9.mp4` is a one-second, 320 × 180 VP9/yuv420p video at 5 fps,
without audio or user media. It was generated with FFmpeg 8.1.2 using:

```sh
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=320x180:rate=5 -t 1 -an -c:v libvpx-vp9 -b:v 0 -crf 42 -row-mt 0 -threads 1 -pix_fmt yuv420p -movflags +faststart synthetic-vp9.mp4
shasum -a 256 synthetic-vp9.mp4
```

SHA-256: `a5721c66bc046ad681c49cd5be3b3a595b859671e1f22e397b37dc4f779a0368`.
The VP9 application journey keeps the real executable search path, including on
Windows, and verifies actual preparation, JPEG decoding, Telegram video payload,
caption, source hash, quota accounting, and scratch cleanup. This is distinct
from the H.264 Windows journey below that deliberately excludes FFmpeg.

The HEVC and AV1 companions have the same one-second 320 × 180/5 fps source.
FFmpeg 8.1.2 generated them with these commands:

```sh
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=320x180:rate=5 -t 1 -an -c:v libx265 -preset ultrafast -x265-params pools=none:frame-threads=1:log-level=error -crf 35 -tag:v hvc1 -pix_fmt yuv420p -movflags +faststart synthetic-hevc.mp4
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=320x180:rate=5 -t 1 -an -c:v libsvtav1 -preset 12 -crf 45 -svtav1-params lp=1 -pix_fmt yuv420p -movflags +faststart synthetic-av1.mp4
```

SHA-256 values:

- `synthetic-hevc.mp4`: `c4ef8aeee69cce46fed51c47db3e5df8e6f76555affe10f4b5bb113134c06ebd`
- `synthetic-av1.mp4`: `906441aae273b918aa287b47affd19ef254f62526ace96c0e32e252bce5e9261`

These fixtures exercise the existing MP4 metadata parser and real FFmpeg codec
support. They do not broaden accepted containers or claim receiving Telegram
clients can play every codec. Windows fallback journeys can explicitly fail or
stall native decoding through a marker read only by the `native-e2e` build;
normal application builds do not accept that fault. The existing H.264 journey
still exercises the actual native Windows decoder without FFmpeg.

Additional lifecycle journeys use the compiled, fixture-owned helper as a
failing or stalled FFmpeg process. Their JPEG bytes are controlled fixtures;
they verify discovery, fallback, errors, cancellation, process containment and
cleanup, rather than actual codec decoding. The sidecar-fallback journeys then
use real FFmpeg for the VP9 thumbnail.

The native E2E journey writes a valid 210 MiB ISO-BMFF `free` box as sparse
padding, either after the early-moov container or before its trailing `moov`.
Media offsets remain unchanged. This exercises real >200 MiB file handling and
metadata seeking; it does not model a long or high-bitrate recording. The test
drives the production media preparation and publication operation through a
native child process. Its controlled Telegram transport reads the actual paced
thumbnail bytes and captures the actual publication, including caption and TL
payload. Assertions independently decode the serialized TL and JPEG. The same
operation handles account fences, cancellation and thumbnail bandwidth holds.
Uploaded main-file part IDs are controlled fixtures; neither the large video
payload nor thumbnail is sent to Telegram. Live Telegram Desktop playback
remains a separate release acceptance check. Temporary decoder staging uses a
private scratch directory unique to the process and is checked for cleanup.
Protected-upload behavior remains covered by the existing encrypted-upload
journeys rather than a test-only bypass in this facade.

The existing Windows H.264 success journeys have an empty executable search
path: successful extraction therefore proves native Windows codec execution
without an FFmpeg fallback. The added VP9/HEVC/AV1 fallback journeys retain PATH
and explicitly fail native decoding in the Windows fixture so real FFmpeg must
decode their thumbnails. Other platform codec journeys also use real FFmpeg.
Unix cancellation, account-switch and deadline journeys substitute an owned
blocking codec executable at the external codec boundary. They exercise the
production process budget, process termination and scratch cleanup; they do not
claim actual codec decoding in those specific lifecycle journeys.
