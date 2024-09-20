#!/usr/bin/env bash

cross build --profile release --target x86_64-linux-android && \
    # cross build --profile release --target i686-linux-android && \
    cross build --profile release --target armv7-linux-androideabi && \
    cross build --profile release --target aarch64-linux-android

mkdir -p android/jniLibs/arm64-v8a/ && \
  cp target/aarch64-linux-android/release/libios_test.so android/jniLibs/arm64-v8a/libios_test.so && \
  mkdir -p android/jniLibs/armeabi-v7a/ && \
    cp target/armv7-linux-androideabi/release/libios_test.so android/jniLibs/armeabi-v7a/libios_test.so && \
  # mkdir -p android/jniLibs/x86/ && \
  #   cp target/i686-linux-android/release/libios_test.so android/jniLibs/x86/libios_test.so && \
  mkdir -p android/jniLibs/x86_64/ && \
    cp target/x86_64-linux-android/release/libios_test.so android/jniLibs/x86_64/libios_test.so

# cargo run -p uniffi-bindgen generate -l kotlin --library target/x86_64-linux-android/release/libios_test.so --out-dir android/kotlin/
cargo run --features=uniffi/cli --bin uniffi-bindgen generate -l kotlin --library target/x86_64-linux-android/release/libios_test.so --out-dir android/kotlin/

