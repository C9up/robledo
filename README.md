# @c9up/robledo

Over-the-air updates for a ream app's mobile build: when only the JS changed,
the app updates itself without a store release.

- `ream android:publish` bundles the app's JS, signs it with your key and
  writes it under `robledo/` in the project;
- the app's ream server serves it (this package's provider);
- the phone downloads it, checks the signature against the key built into
  the app and every file against its digest, and switches to it at the next
  start. An update that crashes before its first screen is abandoned at the
  start after, and the previous one runs again.

Named after the Deep Space Network complex near Madrid, which uplinks
commands and software to spacecraft — as `goldstone` is its Californian
sibling.

## Set up

```bash
ream add @c9up/robledo
ream generate:update-key        # writes robledo.key, prints the public key
```

Keep `robledo.key` out of git; on CI, pass its content as `REAM_UPDATES_KEY`.

```ts
// reamrc.ts
export default defineConfig({
  providers: [() => import('@c9up/robledo/provider')],
  mobile: {
    // … name, id, entry (see `ream android:build`)
    updates: {
      url: 'https://api.example.com/__robledo',
      publicKey: '…',            // what generate:update-key printed
    },
  },
})
```

```ts
// start/routes.ts — from a preload, so the routes exist before the server listens
import robledo from '@c9up/robledo/services/main'
robledo.registerRoutes()
```

```ts
// the app's root component
import { checkForUpdate, markReady } from '@c9up/robledo/mobile'

onMount(() => {
  markReady()             // this update reached its first screen: keep it
  void checkForUpdate()   // a newer one, if any, applies at the next start
})
```

## Publish

```bash
ream android:publish
```

The update is written to `robledo/android/<runtime>/<id>/`, and becomes the
newest once all its files are there. Deploy that directory with the app.

## What a phone accepts

An update is for one **native runtime**: a digest of React Native's version
and every native package's name and version, computed by `ream` at build and
at publish. A phone asks only for its own runtime's updates and refuses any
other: JS built against different native code needs a store release.

What a phone checks before it keeps an update:

- the manifest's Ed25519 signature, against the public key built into the
  app — the server holds no key, so a compromised server can withhold an
  update but not forge one;
- its runtime and platform;
- every file's SHA-256 and size, and that no path leaves the update's
  directory;
- that the update did not crash on this phone before.

Updates are fetched over https; plain http is accepted only to this computer
(`localhost`, `127.0.0.1`, or the emulator's `10.0.2.2`), for development.

## Rolling back

At each start, an update launched for the first time is on trial until the
app calls `markReady()`. One that never does — it crashed, or was killed
before its first screen — is dropped at the next start: the previous update
(or the app's own bundle) runs, and the failed one is not staged again.

## Configuration

`config/robledo.ts`, or `robledo` in the config store:

| Key | Default | |
|---|---|---|
| `directory` | `robledo` | where published updates are, from the app's root |
| `prefix` | `/__robledo` | the routes' prefix |

`mobile.updates.directory` in `reamrc.ts` tells `android:publish` where to
write, when it is not `robledo`.

## Pieces

- `src/server` — the routes: `GET <prefix>/:platform/:runtime/latest` and
  `GET <prefix>/:platform/:runtime/:id/files/*`.
- `src/mobile` — `checkForUpdate`, `markReady`, `updateStatus`.
- `crates/robledo-client` — the phone's side in Rust: verification, staging,
  the launch decision and the rollback, behind a C ABI (iOS will reuse it).
- `android/` — the JSI module and the app-start hook `ream android:*` wires in
  (`reamNative` in package.json).

iOS is not there yet.
