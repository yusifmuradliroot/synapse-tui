# synapse-tui

Terminalde çalışan evrensel AI kod asistanı. Tek binary, ek bağımlılık yok.

**Desteklenen platformlar:** Windows (conhost + Windows Terminal) · Linux · macOS (Intel + Apple Silicon) · Termux (Android)

## Özellikler

- OpenRouter ve NVIDIA NIM sağlayıcıları (düşünen modeller dahil)
- Agentic dosya araçları: okuma/yazma/düzenleme/arama/glob/apply_patch/web/skill/todo
- İzin sistemi: her dosya/shell işleminde `allow once / always allow / deny`
- Oturumlar: `/new /sessions /resume`, otomatik başlık, `/compact` ile bağlam sıkıştırma
- Shell modu (`!`), arka plan işleri, proje hafızası (`MEMORY.md`), `/init` ile proje özeti
- `Ctrl+V` pano yapıştırma, fare + tekerlek, animasyonlu durum yıldızı

## Kurulum

### 1) Hazır binary (önerilen)

[Releases](https://github.com/yusifmuradliroot/synapse-tui/releases/latest) sayfasından işletim sistemine uygun dosyayı indir:

| Platform | Dosya |
|---|---|
| Windows x64 | `synapse-distilled-windows-x64.exe` |
| Linux x64 | `synapse-distilled-linux-x64` |
| Linux ARM64 | `synapse-distilled-linux-arm64` |
| macOS Apple Silicon | `synapse-distilled-macos-arm64` |
| macOS Intel | `synapse-distilled-macos-x64` |
| Termux (Android ARM64) | `synapse-distilled-android-arm64` |

Windows: indir ve çift tıkla (veya PowerShell'den çalıştır).
Linux/macOS: `chmod +x synapse-distilled-*` sonra `./synapse-distilled-*` ile çalıştır.
macOS ilk açılışta engellerse: Sistem Ayarları → Gizlilik ve Güvenlik → yine de aç.

### 2) cargo ile kurulum

Önce [Rust](https://rustup.rs) kurulu olmalı, sonra:

```sh
cargo install --git https://github.com/yusifmuradliroot/synapse-tui
```

Binary `~/.cargo/bin/synapse-distilled` altına kurulur.

### 3) Kaynaktan derleme

```sh
git clone https://github.com/yusifmuradliroot/synapse-tui
cd synapse-tui
cargo build --release
./target/release/synapse-distilled
```

Platform notları:

- **Termux:** `pkg install rust git` yeterli, sonra yukarıdaki adımlar aynen çalışır.
- **Windows:** Rust için MSVC build tools gerekir (`rustup` kurulumu yönlendirir).
- Başka hedef için cross derleme: `cargo build --release --target <hedef>`
  (Windows hedefi için `x86_64-w64-mingw32-gcc` gerekir.)

## API anahtarı

Uygulama ilk açılışta anahtar ister, veya komutla verilir:

```
/key sk-or-...            # OpenRouter (https://openrouter.ai/keys)
/provider nvidia
/key nvapi-...            # NVIDIA NIM (https://build.nvidia.com)
```

Alternatif: `synapse-distilled --key <anahtar> --model <id>` ile başlatma.
Anahtarlar bilgisayarda saklanır: Windows `%APPDATA%\synapse-distilled`,
Linux/macOS/Termux `~/.config/synapse-distilled`.

## Kullanım

| Komut | İşlev |
|---|---|
| `/help` | tüm komutlar |
| `/model` | model seç / değiştir |
| `/provider openrouter\|nvidia` | sağlayıcı değiştir |
| `/settings` | sıcaklık, token limiti, bağlam, izinler |
| `/key` | API anahtarı kaydet |
| `/cd <klasör>` | çalışma klasörünü değiştir |
| `/run <komut>` veya `!` | shell komutu çalıştır |
| `/init` | projeyi özetleyip hafızaya yaz |
| `/compact` | konuşmayı özetleyip bağlamı boşalt |
| `/new /sessions /resume <id>` | oturum yönetimi |
| `/clear /context /tools /thinking` | ekran, bağlam, araçlar, düşünme |
| `/m` `/c` | model / sohbet görünümü |

Kısayollar: `TAB` sohbet odağı · `!` shell modu · `Ctrl+V` yapıştır ·
fare/tekerlek kaydırma · izin ekranında `←→`/`1-3`/`Enter` (`Esc` = reddet) · `q` çıkış.

Güvenlik: ajan yalnızca çalıştığı klasörün (`workspace`) içine yazar.
Başka klasörde çalışmak için programı orada başlat veya `/cd` ile geç.

## Lisans

MIT — bkz. [LICENSE](LICENSE).
