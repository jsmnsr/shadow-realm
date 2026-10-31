# Third-party notices

These crates are resolved for the Windows x64 build and tests. All offer MIT
and Apache-2.0 as alternative licenses; this distribution uses the MIT option.
The shared MIT terms below apply to each crate with its upstream copyright
notice shown here. A dash means the supplied `LICENSE-MIT` has no copyright line.

| Crate | Version | Upstream MIT copyright notice |
| --- | --- | --- |
| anstyle | 1.0.14 | Copyright (c) Individual contributors |
| anyhow | 1.0.104 | - |
| cfg-if | 1.0.5 | Copyright (c) 2014 Alex Crichton |
| clap | 4.6.7 | Copyright (c) Individual contributors |
| clap_builder | 4.6.7 | Copyright (c) Individual contributors |
| clap_lex | 1.1.1 | Copyright (c) Individual contributors |
| fastrand | 2.5.0 | - |
| getrandom | 0.4.3 | Copyright (c) 2018-2026 The rust-random Project Developers; Copyright (c) 2014 The Rust Project Developers |
| once_cell | 1.21.4 | - |
| tempfile | 3.27.0 | Copyright (c) 2015 Steven Allen |
| windows-link | 0.2.1 | Copyright (c) Microsoft Corporation. |
| windows-sys | 0.61.2 | Copyright (c) Microsoft Corporation. |

`tempfile` and its additional dependencies (`cfg-if`, `fastrand`, `getrandom`,
and `once_cell`) are test-only, not linked into the release executable.

When changing dependencies, review their licenses and update this notice.
Packaging checks that every resolved dependency's name and version is listed.

## MIT License

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
SOFTWARE
