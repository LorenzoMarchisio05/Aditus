# Official CEF + swapped libffmpeg: verdict

**Not feasible for CEF on macOS.** The official Spotify CEF build has ffmpeg statically linked into the framework, so there is no `libffmpeg.dylib` to swap. Keep `scripts/fetch-cef.sh` (obs-rust build).

## Evidence (CEF 156.0.8+g426920c, Chromium 156.0.8078.48, macosarm64 minimal, checked 2026-10-28)

- Official `Libraries/` holds only `libcef_sandbox.dylib`, `libvk_swiftshader.dylib`, `libvulkan.dylib`, `vk_swiftshader_icd.json`. No `libffmpeg.dylib`.
- `otool -L` on `Chromium Embedded Framework` lists only its own install name plus system frameworks. No libffmpeg load command, no `LC_RPATH`, and `strings` finds no `libffmpeg`. A dylib that nothing links or `dlopen`s is never loaded.
- Chromium's `FFmpegVideoDecoder`/`FFmpegAudioDecoder` wrappers are inside the binary, so the decoders are linked in (`is_component_ffmpeg=false`).
- The obs-rust build (H.264/AAC works) has the same layout: no libffmpeg, but its framework binary is 11.6 MB larger (249.7 MB vs 238.1 MB). The codecs are compiled in, not loaded.
- Side finding: the framework currently in `~/.local/share/cef` is byte-identical (sha256 `ad664126...`) to the **official** build, so that dev copy has no H.264. The obs-rust binary hashes differently (`f85b5611...`).
- The official archive on cef-builds.spotifycdn.com has the same version string as the pinned one, so the `cef` crate's `archive.json` check would pass. sha256 of the official archive: `c51efd8c...7576` (sha1 `58510b0c...` matches the Spotify index).

## Why Electron/NW.js can do it and CEF can't

Electron and NW.js build with `is_component_build=false is_component_ffmpeg=true`. This produces `libffmpeg.dylib` (install name patched to `@loader_path`, see Electron's `patches/ffmpeg/link_with_loader_path.patch`) and the main binary gets a real load command for it. Swapping then replaces a dylib that is already linked. CEF's official builds use `is_component_ffmpeg=false`, so the binary has no such link.

A second blocker, even if the link existed: `proprietary_codecs=true` also compiles non-ffmpeg code in `//media` (MP4/H.264 demux and parsing, MIME support tables gated by `USE_PROPRIETARY_CODECS`). A Chrome-branded libffmpeg alone only adds the decoders. I did not test this one, because the first blocker already ends it.

## Prebuilt libffmpeg availability (for the record)

[nwjs-ffmpeg-prebuilt](https://github.com/nwjs-ffmpeg-prebuilt/nwjs-ffmpeg-prebuilt) 0.119.2 matches Chromium 156.0.8078.48 and ships `0.119.2-osx-arm64.zip` (avcodec63/avformat63/avutil61). It is unusable here: it is built for NW.js's link layout, and nothing in CEF would load it. Version rule if a swap ever applied: the libffmpeg must come from the exact Chromium major (ideally the exact build), since Chromium's `media/` code calls into ffmpeg's ABI directly.

## What would work

1. Keep obs-rust (current). Risks: built on one contributor's Mac and pinned by hash, no attested build, lags new Chromium releases.
2. Build CEF yourself with `proprietary_codecs=true ffmpeg_branding=Chrome` (CEF automate-git.py, ~100 GB disk, hours; arm64 macOS). Same result as obs-rust, under your control.
3. Hardware decode only: macOS VideoToolbox covers H.264 in Chromium without ffmpeg, but a CEF with `proprietary_codecs=false` still reports H.264 unsupported to the page (CEF issue 3559), and AAC would still be missing. Not a fix.

## Codesigning / notarization

`backend/sign-mac.sh` signs the whole framework with `codesign --options runtime` and entitlements already include `disable-library-validation`. Had a libffmpeg been dropped into `Libraries/`, it would be covered by signing the framework (sign it before the framework, inside out, with the same identity). Notarization has no codec-specific rules. Not tested: no Developer ID certificate here.

## Patent licensing

H.264 and AAC are patent-encumbered (MPEG LA / Via LA pools). Shipping software decoders, whether in libcef or a separate libffmpeg, is the distributor's licensing responsibility in their jurisdiction. This applies equally to the current obs-rust build.

## Script

`scripts/fetch-cef-official.sh <dir> [libffmpeg.dylib]` downloads the official archive (sha256 verified), lays it out like `fetch-cef.sh`, and writes `archive.json`. It refuses a given libffmpeg when the framework has no libffmpeg load command, which is the case today. Tested: download and verification OK, refusal OK. Not wired into CI.

## Unverified

- Playback was not tested in the app (cannot play video here).
- The claim that the obs-rust size delta is codecs is an inference, not a symbol diff (binaries are stripped).
- Whether Chromium's Mac MIME/demux gates alone would block H.264 with an ffmpeg-only swap (moot).
