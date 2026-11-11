# Restorix — deleted file recovery for Windows and macOS

Restorix finds deleted files on internal drives, USB sticks, memory cards and disk images, lets you
search and filter them, previews them, and restores them to another drive.

It is a [Tauri](https://tauri.app) desktop app: a React interface on top of a Rust recovery engine.
Disks are always opened **read-only**. Data is only written to the destination folder you pick, and for raw
recoveries Restorix refuses a destination on the same physical disk.

## What it can recover

| Method | Where it works | Names & folders | Deletion date |
|---|---|---|---|
| **Trash / Recycle Bin** | Windows `$Recycle.Bin`, macOS `~/.Trash` and `/Volumes/*/.Trashes` | ✅ (Windows: original path) | ✅ |
| **Snapshots & backups** | Windows Volume Shadow Copies, macOS APFS local snapshots, Time Machine | ✅ | "after snapshot date" |
| **File system scan** | NTFS (MFT + USN journal), FAT12/16/32, exFAT, HFS+ (catalog leftovers + journal), APFS (stale B-tree nodes) | ✅ | NTFS only (from `$UsnJrnl`) |
| **Deep scan (carving)** | Any disk, even unformatted: JPEG, PNG, GIF, BMP, WebP, TIFF, camera RAW (CR2/NEF/ARW/DNG/PEF), HEIC/AVIF, CR3, MP4/MOV/M4V/3GP, AVI | ❌ (named by type + offset) | ❌ (photo/video creation date from EXIF / `mvhd`) |

Every result gets a **recoverability** rating, which tells you whether the file's data is still in free space:

* **Good**: all clusters/blocks are still unallocated.
* **Partial**: some were reused, or the end of a carved file was not found.
* **Overwritten**: the space now belongs to other files.

### Search & filter

* Name search with wildcards (`*.jpg`, `IMG_20??*`)
* Deletion date range and creation date range, optionally including files whose date is unknown
* Type (Photos / Videos / Audio / Documents / Archives / Other) and individual extensions
* Recovery method, recoverability, size range
* Folder tree rebuilt from the original paths. Click a folder to filter to it, then restore the whole folder.

### Restore

* Restore selected files, a folder, or everything that matches the current filters
* Optionally keep the original folder structure
* Preserves the modification time and writes a CSV report next to the restored files
* Preview before restoring: images, RAW (embedded JPEG), HEIC (macOS), video, text, and a hex view
  that shows whether the data was zeroed by SSD TRIM

## Important limits (read before relying on it)

* **SSDs with TRIM** (all modern Macs and most Windows laptops): the drive erases deleted blocks
  within seconds to minutes. On these drives, *Trash* and *Snapshots* are what usually work. File-system
  and carving results may show **Good** metadata but zeroed content. The preview's hex view shows this.
* **Encrypted volumes** (FileVault, BitLocker): raw reads return ciphertext, so the file-system and carving
  methods can't work. Trash and snapshots still do.
* **Deletion dates** are stored only by NTFS (USN journal) and the Recycle Bin / Trash. FAT, exFAT, HFS+ and
  APFS do not record them.
* **Carving** assumes a file is stored contiguously. Fragmented videos come back partially.
* **FAT32**: deleting a file erases the first character of its 8.3 name. When there is no long name it shows as `_`.
* **APFS / HFS+** metadata recovery is best-effort. It depends on stale metadata blocks still being on
  disk; macOS often wipes HFS+ catalog records right away. Carving still finds the photos and videos.

## Permissions

* **Windows**: the app asks for Administrator rights at launch (UAC). Raw volume and disk access needs them.
* **macOS**: a raw scan asks for your administrator password through Apple's `authopen`. For the Trash
  and Time Machine, give Restorix **Full Disk Access** in System Settings › Privacy & Security.
  Mounting APFS snapshots asks for your password once.

## Build

Requirements: Rust (stable), Node.js 18+, and on Windows the WebView2 runtime (bundled installer downloads it).

```bash
npm install
npm run tauri dev          # run in development
npm run tauri build        # build installers
```

* **macOS** produces `target/release/bundle/macos/Restorix.app`. If tauri's styled DMG step fails
  (it drives Finder), run `scripts/make-dmg.sh` to package a plain DMG.
* **Windows** produces `target/release/bundle/msi/*.msi` and `target/release/bundle/nsis/*-setup.exe`.
  Run `npm run tauri dev` from an *Administrator* terminal, because the app's manifest requires elevation.
* **Both at once**: push a tag `v*` (or run the workflow manually) and `.github/workflows/build.yml`
  builds the Windows installers and a universal macOS DMG.

The unsigned macOS app shows a Gatekeeper warning on first launch (right-click › Open). For
distribution, sign and notarize it (set `APPLE_SIGNING_IDENTITY` etc. for `tauri build`).

## Windows: build the `.exe` installer and run Restorix

There are two ways to get `Restorix_1.0.0_x64-setup.exe`. Building on Windows is the most direct;
GitHub Actions needs no tools on your PC.

### Option 1 — build on a Windows 10/11 PC

1. **Install the build tools** (one time):
   * **Microsoft C++ Build Tools**: download from
     <https://visualstudio.microsoft.com/visual-cpp-build-tools/>, run it, tick
     **"Desktop development with C++"** and install.
   * **Rust**: download and run `rustup-init.exe` from <https://rustup.rs> and accept the defaults.
   * **Node.js 18 or newer** (LTS): <https://nodejs.org>.
   * **Git**: <https://git-scm.com/download/win>.
   * WebView2 is already part of Windows 10 (21H2+) and 11.

   Close and reopen PowerShell afterwards so the new tools are on the PATH. Check them with:

   ```powershell
   rustc --version; cargo --version; node --version; npm --version
   ```

2. **Get the code**:

   ```powershell
   git clone https://github.com/elbauomy/Restorix.git
   cd Restorix
   ```

3. **Build the installers**:

   ```powershell
   npm install
   npm run tauri build
   ```

   The first build downloads and compiles dependencies and takes several minutes. When it finishes:

   | File | What it is |
   |---|---|
   | `target\release\bundle\nsis\Restorix_1.0.0_x64-setup.exe` | **Installer `.exe`** — give this to users |
   | `target\release\bundle\msi\Restorix_1.0.0_x64_en-US.msi` | MSI installer (for IT / group-policy deployment) |
   | `target\release\restorix.exe` | The program itself, without an installer (portable) |

4. **Install and run**: double-click `Restorix_1.0.0_x64-setup.exe`. Because the build is not
   code-signed, Windows SmartScreen may show *"Windows protected your PC"* — click **More info ›
   Run anyway**. Start Restorix from the Start menu and click **Yes** when Windows asks for
   Administrator permission (raw disk access requires it).

To run it straight from source while developing, open PowerShell **as Administrator**, then:

```powershell
cd Restorix
npm run tauri dev
```

### Option 2 — let GitHub build it

1. On GitHub, open the repository's **Actions** tab › **Build installers** › **Run workflow**
   (or push a tag such as `v1.0.0`).
2. When the run turns green (about 10–15 minutes), open it and download the
   **`restorix-windows-latest`** artifact.
3. Unzip it; the installer is in `nsis\Restorix_1.0.0_x64-setup.exe`. Install it as in step 4 above.

The same workflow also produces a universal macOS DMG (artifact `restorix-macos-latest`).

### Troubleshooting on Windows

* **`link.exe not found` / `MSVC` errors** — the C++ Build Tools are missing or "Desktop
  development with C++" was not ticked. Re-run the installer and add it.
* **`'npm' is not recognized` / `'cargo' is not recognized`** — open a *new* PowerShell window after
  installing Node.js / Rust.
* **The app says it cannot open a drive** — it was not started as Administrator. Right-click
  Restorix › **Run as administrator**.
* **Error 740 ("requires elevation") with `npm run tauri dev`** — run PowerShell as Administrator.
* Restore recovered files to a **different drive** than the one you are scanning.

### Command-line engine

The engine also ships a CLI, useful for scripting and testing:

```bash
cargo run --release -p restorix-engine --bin restorix-cli -- devices
cargo run --release -p restorix-engine --bin restorix-cli -- scan  /path/to/card.img
cargo run --release -p restorix-engine --bin restorix-cli -- restore /path/to/card.img /Volumes/Backup/out
```

## Tests

```bash
cargo test -p restorix-engine                 # unit tests (parsers, dates, carving)
scripts/make-test-images.sh test-images       # macOS: FAT32/exFAT/HFS+/APFS images with deleted files
python3 scripts/make_ntfs_image.py test-images/ntfs.img "test-images/samples/beach photo.jpg"
RESTORIX_TEST_IMAGE=test-images/fat32.dmg cargo test -p restorix --test ipc -- --nocapture   # full app IPC path
```

## Project layout

```
engine/            Rust recovery engine (library + restorix-cli)
  src/io.rs          read-only device/image access (aligned raw reads, macOS authopen)
  src/partition.rs   MBR/GPT discovery
  src/fs/            ntfs.rs, fat.rs, exfat.rs, hfsplus.rs, apfs.rs
  src/carve/         signature carving + structure-aware length detection
  src/trash.rs       Recycle Bin / Trash
  src/snapshots.rs   VSS, APFS snapshots, Time Machine
  src/query.rs       search, filters, folder tree
  src/restore.rs     restore + CSV report
  src/preview.rs     previews
src-tauri/         Tauri app (commands in src/lib.rs, Windows elevation manifest)
src/               React UI
scripts/           test-image builders, DMG packager
```
