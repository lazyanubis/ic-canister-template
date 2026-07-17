#!/bin/bash

set -euo pipefail

RUST_BACKTRACE=1 cargo test upload -- --ignored --nocapture
