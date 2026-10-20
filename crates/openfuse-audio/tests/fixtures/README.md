# Independent audio fixture

`sine.mp3` is an original synthetic 440 Hz tone, 100 ms, stereo, 44.1 kHz,
encoded with FFmpeg/libmp3lame. It contains no Fender audio or artwork.
Regenerate from the repository root (requires FFmpeg only for research):

```sh
ffmpeg -f lavfi -i 'sine=frequency=440:duration=0.1:sample_rate=44100' \
  -ac 2 -codec:a libmp3lame -b:a 128k -map_metadata -1 \
  crates/confuse-audio/tests/fixtures/sine.mp3
```

The test verifies actual decoding, channel mapping, duration and signal level.
It does not claim bit-identical output to the discontinued Silverlight decoder.
