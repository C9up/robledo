#pragma once

#include <cstddef>
#include <cstdint>

// robledo-client's C ABI (crates/robledo-client/src/ffi.rs).
extern "C" {

struct RobledoBuf {
  uint8_t* ptr;
  size_t len;
  size_t cap;
};

void robledo_buf_free(RobledoBuf buf);
int32_t robledo_open(const uint8_t* dir, size_t dirLen, const uint8_t* config, size_t configLen, RobledoBuf* out);
int32_t robledo_verify(
    const uint8_t* manifest, size_t manifestLen, const uint8_t* signature, size_t signatureLen, RobledoBuf* out);
int32_t robledo_receive(const uint8_t* path, size_t pathLen, const uint8_t* data, size_t dataLen, RobledoBuf* out);
int32_t robledo_finish(RobledoBuf* out);
int32_t robledo_ready(RobledoBuf* out);
int32_t robledo_status(RobledoBuf* out);
}
