# Third-party notices

Apature's own code is MIT OR Apache-2.0 (see `LICENSE-MIT` and `LICENSE-APACHE`).
Crate dependencies and their licences are listed in `docs/THIRD_PARTY.md`.
This file records code that was ported or adapted from other projects.

## extract-raw-preview

- Project: <https://github.com/radenkovic/extract-raw-preview>
- Used in: `crates/apature-preview/src/embedded.rs` and `jpeg_info.rs`
- What was ported: the per-format rules for finding the camera's embedded
  JPEG preview: the Fujifilm RAF header offsets, the Canon CR3 box walk and
  CRW heap records, the Sigma X3F section directory, the Panasonic RW2
  `JpgFromRaw` tag, the Olympus and Pentax maker-note preview tags, the
  "largest preview by default" choice with a byte ceiling, and rejecting
  lossless (SOF3) JPEG streams, which are raw sensor data and not previews.
  It was rewritten in Rust; no JavaScript is shipped.
- Licence: MIT, reproduced below.

```text
MIT License

Copyright (c) 2026 Dan Radenkovic

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
