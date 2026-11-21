# Link robledo into a React Native app's native library (`appmodules`):
#
#   include(${ROBLEDO_DIR}/android/robledo.cmake)
#   robledo_link(${CMAKE_PROJECT_NAME})
#
# `ream android:*` does this for an app that depends on @c9up/robledo, builds
# the Rust client into REAM_RUST_DIR/<abi>/ and registers NativeRobledo in the
# app's module provider.

set(ROBLEDO_ANDROID_DIR ${CMAKE_CURRENT_LIST_DIR})

if(DEFINED REAM_RUST_DIR)
  set(ROBLEDO_CLIENT_LIB ${REAM_RUST_DIR}/${ANDROID_ABI}/librobledo_client.a)
else()
  set(ROBLEDO_CLIENT_LIB ${ROBLEDO_ANDROID_DIR}/libs/${ANDROID_ABI}/librobledo_client.a)
endif()

function(robledo_link target)
  target_sources(${target} PRIVATE ${ROBLEDO_ANDROID_DIR}/src/main/cpp/NativeRobledo.cpp)
  target_include_directories(${target} PUBLIC ${ROBLEDO_ANDROID_DIR}/src/main/cpp)
  add_library(robledo_client STATIC IMPORTED)
  set_target_properties(robledo_client PROPERTIES IMPORTED_LOCATION ${ROBLEDO_CLIENT_LIB})
  target_link_libraries(${target} robledo_client)
endfunction()
