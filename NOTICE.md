# Third-party notices

Offspring is released under the [MIT License](./LICENSE).

## FFmpeg

Offspring does **not** bundle FFmpeg. On first install, the installer offers to
download the latest FFmpeg **LGPL essentials** build from
[gyan.dev](https://www.gyan.dev/ffmpeg/builds/) into
`%LOCALAPPDATA%\Offspring\ffmpeg\`. Users may also point Offspring at a
pre-existing FFmpeg installation via the in-app Settings panel.

FFmpeg is © the FFmpeg developers and is licensed under the
[GNU Lesser General Public License, version 2.1 or later](https://www.ffmpeg.org/legal.html).
Source code for FFmpeg is available at <https://ffmpeg.org/download.html>.

Because Offspring invokes FFmpeg as a separately-installed executable (not
statically or dynamically linked), the LGPL does not propagate to Offspring's
own source code. Offspring's MIT license covers only the Rust + Svelte code in
this repository.

## Rust dependencies

Rust crates used by the app are listed in [`src-tauri/Cargo.toml`](./src-tauri/Cargo.toml).
Each crate retains its upstream license. A complete attribution list can be
generated with `cargo about` if a distribution requires one.

## Icon assets

Icons under [`src-tauri/icons/`](./src-tauri/icons/) are original work
© 2026 Rolando Barry, released under the same MIT license as the app.
