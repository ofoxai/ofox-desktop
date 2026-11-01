# Audio fixtures

`tone-16k-mono.mp3` is a synthetic 440 Hz tone, one second, mono, 16 kHz,
32 kbit/s MP3. It contains no speech, personal data or provider output. Generated
for these tests and dedicated to CC0-1.0; no third-party sound recording is used.

Regenerate with an FFmpeg installation supporting libmp3lame:

```sh
ffmpeg -nostdin -v error -f lavfi -i 'sine=frequency=440:sample_rate=16000:duration=1' -ac 1 -codec:a libmp3lame -b:a 32k -map_metadata -1 -id3v2_version 0 -write_xing 1 tone-16k-mono.mp3
```

Tests create PCM WAV bytes directly, so no encoder is needed to run them.
