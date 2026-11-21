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
int32_t robledo_stage(
    const uint8_t* manifest,
    size_t manifestLen,
    const uint8_t* signature,
    size_t signatureLen,
    size_t count,
    const uint8_t* const* paths,
    const size_t* pathLens,
    const uint8_t* const* data,
    const size_t* dataLens,
    RobledoBuf* out);
int32_t robledo_ready(RobledoBuf* out);
int32_t robledo_status(RobledoBuf* out);
}
