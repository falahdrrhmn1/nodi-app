# Nodi — Mark III

Aplikasi catatan **native Windows** yang ringan: Rust + Win32 + Microsoft Rich Edit.
Tanpa Electron, Tauri, WebView, browser, Node.js, atau server lokal.

Tampilan mengikuti Notepad++ (menu klasik, toolbar kecil, tab), sidebar pohon
catatan mengikuti KeepNote (catatan boleh punya anak catatan), dan editornya
WYSIWYG (Ctrl+B langsung tebal, bukan `**teks**`).

## Menjalankan

### Cara biasa (portable)

```powershell
cd E:\nodi-app\mark-3
powershell -ExecutionPolicy Bypass -File scripts\build.ps1
```

Hasilnya `E:\nodi-app\mark-3\dist\Nodi\nodi.exe`. Klik dua kali untuk membuka
(boleh dibuat shortcut ke Desktop). Catatan disimpan di sebelahnya:
`dist\Nodi\data\`. Folder `dist\Nodi` bisa dipindah ke drive/folder lain
utuh-utuh beserta datanya.

### Saat pengembangan

```powershell
cargo run -p nodi-windows            # build debug lalu jalankan
```

Exe yang berjalan dari `target\debug` atau `target\release` memakai
`E:\nodi-app\mark-3\data\` (bukan folder `target`, supaya `cargo clean` tidak
menghapus catatan). Lokasi data bisa dipaksa dengan env `NODI_DATA_DIR`.

### Prasyarat build

- Rust di `E:\Software\Rust` (`CARGO_HOME`, `RUSTUP_HOME` sudah diset).
- Toolchain `stable-x86_64-pc-windows-gnu` (dipin di `rust-toolchain.toml`;
  linker MinGW ikut di dalamnya, **tidak perlu** Visual Studio Build Tools).
- `windres` untuk ikon + manifest: dicari di PATH lalu
  `E:\Software\C++ Compiler\bin\windres.exe`.

## Lokasi data

```
data/
  tree.json            struktur pohon (id, judul, ikon, parent, urutan, trash)
  session.json         tab terbuka, posisi kursor, ukuran jendela/sidebar
  notes/<id>.rtf       isi catatan (RTF: format, gambar, tabel ikut di dalamnya)
  notes/<id>.txt       teks polos untuk pencarian
  notes/<id>.rtf.prev  versi sebelumnya (1 generasi)
  backups/<waktu>/     snapshot otomatis harian, disimpan 20 terakhir
  recovery/            file sisa crash / catatan yang dihapus permanen
```

Semua penulisan atomik: tulis `*.tmp` → flush ke disk → simpan versi lama ke
`.prev` → rename. Status bar hanya menampilkan **Saved** setelah penulisan
benar-benar berhasil; kalau gagal tampil **Save Failed**, isi tetap di editor,
dan Nodi mencoba ulang tiap 5 detik.

## Struktur kode

```
crates/nodi-core     logika murni Rust (tanpa Win32): pohon, penyimpanan atomik,
                     autosave writer thread, backup, pencarian, link.
                     Bisa dipakai ulang untuk Android nanti.
crates/nodi-windows  UI Win32: jendela, menu, toolbar, TreeView, tab, Rich Edit.
scripts/             build.ps1, e2e.ps1 (uji end-to-end), smoke.ps1 (ukur startup/RAM)
docs/                ARCHITECTURE.md, SHORTCUTS.md, STATUS.md
```

## Pengujian

```powershell
cargo test                                   # unit + integration (nodi-core)
pwsh -File scripts\e2e.ps1                   # end-to-end terhadap nodi.exe asli
pwsh -File scripts\smoke.ps1                 # waktu startup, RAM, screenshot
```

Lihat `docs/STATUS.md` untuk daftar fitur yang sudah/belum selesai beserta hasil uji.
