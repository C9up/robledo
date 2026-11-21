#include "NativeRobledo.h"

#include <jni.h>

#include <optional>
#include <string>
#include <vector>

#include "robledo_client.h"

namespace facebook::react {

namespace {

std::string take(RobledoBuf buf) {
  std::string s(reinterpret_cast<const char*>(buf.ptr), buf.len);
  robledo_buf_free(buf);
  return s;
}

/** A Uint8Array (any typed-array view) or an ArrayBuffer, as bytes. */
struct Bytes {
  std::optional<jsi::ArrayBuffer> buffer;
  size_t offset = 0;
  size_t length = 0;
};

Bytes bytesOf(jsi::Runtime& rt, jsi::Object object) {
  Bytes out;
  if (object.isArrayBuffer(rt)) {
    out.buffer.emplace(object.getArrayBuffer(rt));
    out.length = out.buffer->size(rt);
    return out;
  }
  jsi::Value inner = object.getProperty(rt, "buffer");
  jsi::Value byteOffset = object.getProperty(rt, "byteOffset");
  jsi::Value byteLength = object.getProperty(rt, "byteLength");
  if (!inner.isObject() || !byteOffset.isNumber() || !byteLength.isNumber() ||
      !inner.getObject(rt).isArrayBuffer(rt)) {
    throw jsi::JSError(rt, "[robledo] stage expects ArrayBuffers or Uint8Arrays");
  }
  out.buffer.emplace(inner.getObject(rt).getArrayBuffer(rt));
  out.offset = static_cast<size_t>(byteOffset.getNumber());
  out.length = static_cast<size_t>(byteLength.getNumber());
  if (out.offset > out.buffer->size(rt) || out.length > out.buffer->size(rt) - out.offset) {
    throw jsi::JSError(rt, "[robledo] a view runs past its buffer");
  }
  return out;
}

} // namespace

NativeRobledo::NativeRobledo(std::shared_ptr<CallInvoker> jsInvoker) : NativeRobledoCxxSpec(std::move(jsInvoker)) {}

std::string NativeRobledo::status(jsi::Runtime& rt) {
  RobledoBuf out{};
  int32_t code = robledo_status(&out);
  std::string text = take(out);
  if (code != 0) {
    throw jsi::JSError(rt, "[robledo] " + text);
  }
  return text;
}

std::string NativeRobledo::stage(
    jsi::Runtime& rt, std::string manifest, std::string signature, jsi::Array paths, jsi::Array files) {
  size_t count = paths.size(rt);
  if (files.size(rt) != count) {
    throw jsi::JSError(rt, "[robledo] stage needs as many files as paths");
  }
  std::vector<std::string> pathStrings;
  std::vector<Bytes> buffers;
  pathStrings.reserve(count);
  buffers.reserve(count);
  for (size_t i = 0; i < count; i++) {
    pathStrings.push_back(paths.getValueAtIndex(rt, i).asString(rt).utf8(rt));
    buffers.push_back(bytesOf(rt, files.getValueAtIndex(rt, i).asObject(rt)));
  }
  std::vector<const uint8_t*> pathPtrs;
  std::vector<size_t> pathLens;
  std::vector<const uint8_t*> dataPtrs;
  std::vector<size_t> dataLens;
  for (size_t i = 0; i < count; i++) {
    pathPtrs.push_back(reinterpret_cast<const uint8_t*>(pathStrings[i].data()));
    pathLens.push_back(pathStrings[i].size());
    dataPtrs.push_back(buffers[i].buffer->data(rt) + buffers[i].offset);
    dataLens.push_back(buffers[i].length);
  }
  RobledoBuf out{};
  int32_t code = robledo_stage(
      reinterpret_cast<const uint8_t*>(manifest.data()),
      manifest.size(),
      reinterpret_cast<const uint8_t*>(signature.data()),
      signature.size(),
      count,
      pathPtrs.data(),
      pathLens.data(),
      dataPtrs.data(),
      dataLens.data(),
      &out);
  std::string text = take(out);
  if (code != 0) {
    throw jsi::JSError(rt, text);
  }
  return text;
}

void NativeRobledo::ready(jsi::Runtime& rt) {
  RobledoBuf out{};
  int32_t code = robledo_ready(&out);
  std::string text = take(out);
  if (code != 0) {
    throw jsi::JSError(rt, "[robledo] " + text);
  }
}

} // namespace facebook::react

// The app's start (com.c9up.robledo.Robledo.bundleFile): open the store with
// the config the build wrote; the bundle to launch, or null for the app's own.
extern "C" JNIEXPORT jstring JNICALL Java_com_c9up_robledo_Robledo_nativeOpen(
    JNIEnv* env, jclass, jstring dir, jbyteArray config) {
  const char* dirChars = env->GetStringUTFChars(dir, nullptr);
  std::string dirString(dirChars);
  env->ReleaseStringUTFChars(dir, dirChars);
  jsize configLen = env->GetArrayLength(config);
  std::vector<uint8_t> configBytes(static_cast<size_t>(configLen));
  if (configLen > 0) {
    env->GetByteArrayRegion(config, 0, configLen, reinterpret_cast<jbyte*>(configBytes.data()));
  }
  RobledoBuf out{};
  int32_t code = robledo_open(
      reinterpret_cast<const uint8_t*>(dirString.data()),
      dirString.size(),
      configBytes.data(),
      configBytes.size(),
      &out);
  std::string text(reinterpret_cast<const char*>(out.ptr), out.len);
  robledo_buf_free(out);
  if (code != 0) {
    jclass error = env->FindClass("java/lang/IllegalStateException");
    env->ThrowNew(error, ("[robledo] " + text).c_str());
    return nullptr;
  }
  return text.empty() ? nullptr : env->NewStringUTF(text.c_str());
}
