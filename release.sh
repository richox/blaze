#!/usr/bin/env bash

RUSTFLAGS='-C target-cpu=native' cargo +nightly build --release --features=mm
