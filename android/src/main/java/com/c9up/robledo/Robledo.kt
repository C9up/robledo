package com.c9up.robledo

import android.content.Context
import android.util.Log
import java.io.File
import java.io.IOException

/**
 * robledo at the app's start. `ream android:*` calls [bundleFile] from the
 * generated MainApplication (this package declares it as its
 * `bundleProvider`): it opens the update store with the config the build
 * wrote (`assets/ream-updates.json`: the server, the public key, the native
 * runtime), rolls back an update that never reported ready, promotes a staged
 * one, and returns the bundle to launch — or null for the one inside the app.
 */
object Robledo {
  private const val CONFIG = "ream-updates.json"

  @JvmStatic external fun nativeOpen(dir: String, config: ByteArray): String?

  @JvmStatic
  fun bundleFile(context: Context): String? {
    val config = try {
      context.assets.open(CONFIG).use { it.readBytes() }
    } catch (e: IOException) {
      // Built without `mobile.updates`: nothing to open, the app's own bundle runs.
      return null
    }
    // The JSI module and the Rust client live in the app's native library.
    com.facebook.soloader.SoLoader.loadLibrary("appmodules")
    return try {
      nativeOpen(File(context.filesDir, "robledo").absolutePath, config)
    } catch (e: IllegalStateException) {
      // A broken store must not keep the app from starting: its own bundle runs.
      Log.e("ROBLEDO", e.message ?: "robledo could not open its store", e)
      null
    }
  }
}
