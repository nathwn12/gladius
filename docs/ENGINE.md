# Engine pin record (obscura)

Gladius ships no engine binary in git. The engine is acquired at provision time by
`scripts/fetch-engine.ps1` into `bin\` beside `gladius.exe`.

## Upstream

- Project: https://github.com/h4ckf0r0day/obscura (Apache-2.0, Rust + V8 + CDP)
- Release: v0.2.4
- Asset: `obscura-x86_64-windows.zip`
- Asset URL: https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.4/obscura-x86_64-windows.zip

## Pin

- Zip sha256: `8e5428bf8c101f1439c53a9dcb52d7d09bb22599206b49f6420028d93d01189b`
- Zip size: `68520432` bytes
- Zip contents: exactly two files at the root, `obscura.exe` and `obscura-worker.exe`.
- `obscura.exe`: size `78702592`, sha256 `65B9DCADE8B425FEF7C0ED56DFAADBA56C3BA990E572A8B1B0CD8B0226932C0E`
- `obscura-worker.exe`: size `72981504`, sha256 `D599456C0E08D9333C5B16F7A6307436E6B4A0295FBCE4A605251F2DFCAD93FA`

## Why bin\ and never committed

`bin\` is gitignored (`/bin` in `.gitignore`). Engine binaries are large
(~150 MB for the pair) and platform-specific; they do not belong in the repo.
Provision them locally with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-engine.ps1
```

The script is idempotent: if both exes already match the pin it prints
`already present, verified` and changes nothing. `scripts\fetch-engine.ps1`
accepts an optional `-SourcePath <zip>` that skips the download and verifies a
local zip instead (useful for offline or test use); the default path always
downloads the pinned release asset.

## Console-subsystem requirement

Both exes are PE32+ (optional-header magic `0x020B`) with Subsystem 3
(Windows console). They can allocate a visible console window, so gladius must
spawn them with `CREATE_NO_WINDOW` (`0x08000000`). Spawning is covered by a
separate change; this document only records the requirement.

## No-DLL-bundling fact

No DLLs ship beside the two exes; the zip contains only the two exes at the
root and nothing else is required at runtime.

## License-file observation

The zip contains no LICENSE file even though the upstream project is
Apache-2.0. Keep the upstream URL and release tag in this record so the license
terms remain traceable.

## Re-pinning a future version

1. Update the constants at the top of `scripts/fetch-engine.ps1`
   (asset URL, zip sha256 + size, both exe sha256s + sizes).
2. Delete `bin\obscura.exe` and `bin\obscura-worker.exe` (or provision on a
   clean checkout) and re-run `scripts\fetch-engine.ps1`.
3. Update this document with the new release, URL, hashes, and sizes.
4. Commit the script and this document. Never commit `bin\`.
