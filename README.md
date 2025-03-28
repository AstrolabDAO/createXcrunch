# `createXcrunch` (WebGPU Fork)

[![👮‍♂️ Sanity checks](https://github.com/HrikB/createXcrunch/actions/workflows/checks.yml/badge.svg)](https://github.com/HrikB/createXcrunch/actions/workflows/checks.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/license/mit)

This is a divergent fork of the original [`createXcrunch`](https://github.com/HrikB/createXcrunch) that uses WebGPU instead of OpenCL to enable cross-platform support. The original OpenCL implementation had compatibility issues with AMD GPUs, Metal-based MacOS, and DirectX-based Windows systems.

`createXcrunch` is a [Rust](https://www.rust-lang.org)-based program designed to efficiently find _zero-leading_, _zero-containing_, or _pattern-matching_ addresses for the [CreateX](https://github.com/pcaversaccio/createx) contract factory using GPU acceleration.

## Key Improvements in this Fork

- **Cross-platform compatibility** - Works consistently across NVIDIA, AMD, Intel GPUs
- **Native support for macOS** - Uses Metal backend on Apple Silicon and Intel Macs
- **Improved Windows support** - No dependency on specific GPU vendor drivers
- **Enhanced pattern matching** - Support for complex patterns like "ABCD...EF"
- **Optimized performance** - Efficient batch processing of results

## Installation

1. **Clone the Repository**

```console
git clone <your-repository-url>
cd contracts/tools/createXcrunch
```

2. **Build the Project**

```console
cargo build --release
```

## Example Setup on [Vast.ai](https://vast.ai)

#### Update Linux

```console
sudo apt update && sudo apt upgrade
```

#### Install `build-essential` Packages

> We need the GNU Compiler Collection (GCC) later.

```console
sudo apt install build-essential
```

#### Install CUDA Toolkit

> `createXcrunch` uses [OpenCL](https://en.wikipedia.org/wiki/OpenCL) which is natively supported via the NVIDIA OpenCL extensions.

```console
sudo apt install nvidia-cuda-toolkit
```

#### Install Rust

> Enter `1` to select the default option and press the `Enter` key to continue the installation. Restart the current shell after completing the installation.

```console
curl https://sh.rustup.rs -sSf | sh
```

#### Build `createXcrunch`

```console
git clone https://github.com/HrikB/createXcrunch.git
cd createXcrunch
cargo build --release
```

🎉 Congrats, now you're ready to crunch your salt(s)!

## Usage

```console
./target/release/createxcrunch create3 --caller 0x88c6C46EBf353A52Bdbab708c23D0c81dAA8134A --matching ABCD...EF
```

You can specify different pattern types:
- Simple leading pattern: `BB`
- Multiple repeating bytes: `BBBB`
- Complex patterns with prefix and suffix: `ABCD...EF`

Use the `--help` flag for a full overview of all features:

```console
./target/release/createxcrunch create3 --help
```

## Local Development

We recommend using [`cargo-nextest`](https://nexte.st) as test runner for this repository. To install it on a Linux `x86_64` machine, invoke:

```console
curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C ${CARGO_HOME:-~/.cargo}/bin
```

Afterwards you can run the tests via:

```console
cargo nextest run
```

## Contributions

PRs welcome!

## Acknowledgements

- Original [createXcrunch](https://github.com/HrikB/createXcrunch)
- [`create2crunch`](https://github.com/0age/create2crunch)
- [Function Selection Miner](https://github.com/Vectorized/function-selector-miner)
- [`CreateX` – A Trustless, Universal Contract Deployer](https://github.com/pcaversaccio/createx)

## Pattern Matching

This fork enhances the pattern matching capabilities with support for complex patterns:

- **Leading patterns**: Find addresses starting with specific hex values
- **Trailing patterns**: Find addresses ending with specific hex values  
- **Complex patterns**: Use the format `PREFIX...SUFFIX` to find addresses that match both criteria

Example patterns:
- `BB` - Addresses starting with the byte value 0xBB
- `BBBB` - Addresses starting with multiple 0xBB bytes
- `CAFE...42` - Addresses starting with 0xCAFE and ending with 0x42
