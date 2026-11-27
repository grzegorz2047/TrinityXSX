# Poradnik użytkownika TrinityXSX (PS5)

Prosta instrukcja krok po kroku: jak zainstalować **TrinityXSX (`PPSA83601`)**, jak wgrywać własne gry **Xbox Series X|S** oraz **PC / GOG (DRM-free)**, jak obsługiwać sterowanie DualSense (w tym przełącznik Pad ⇄ Mysz i klawiatura), gdzie są pliki konfiguracyjne oraz jak zgłaszać błędy.

> **Ważne:** TrinityXSX **nie zawiera żadnych gier, kluczy, BIOS-u ani plików SDK Sony**. Musisz użyć własnej, rozpakowanej kopii gry z Xbox Series X|S (GDK / Scarlett) lub własnych, wolnych od DRM gier PC / instalatorów offline z GOG.

---

## 1. Czego potrzebujesz

- PS5 z działającym homebrew (**ShadowMountPlus** oraz **elfldr**, np. etaHEN + kstuff).
- Klienta FTP na komputerze (FileZilla, WinSCP lub skrypt `tools/ftp_put_tree.py`) — lub **ProsperoStore** bezpośrednio na konsoli.
- Dysku zewnętrznego/rozszerzonego pod `/mnt/ext1` (lub pamięci wewnętrznej `/data` / pendrive'a `/mnt/usb0`).
- Własnych gier:
  - **Xbox Series X|S:** rozpakowany folder gry zawierający `MicrosoftGame.config` (Scarlett) i wskazany `.exe`, **albo**
  - **PC / GOG (DRM-free):** instalator offline GOG (`setup_*.exe` + pliki `.bin`) **lub** już rozpakowany folder z grą PC (32-bit lub 64-bit).

---

## 2. Instalacja TrinityXSX na PS5

### Opcja A — ProsperoStore (najszybsza)
1. Otwórz **[ProsperoStore](https://prosperostore.net/app/PPSA83601/)** na PS5, wyszukaj **TrinityXSX (`PPSA83601`)** i zainstaluj paczkę `PPSA83601.zip`.
2. Uruchom **TrinityXSX** z ekranu głównego PS5.

### Opcja B — Samodzielny obraz `.ffpkg` (ShadowMountPlus)
1. Pobierz najnowszy plik **`PPSA83601-vX.Y.Z.ffpkg.zip`** ze strony [GitHub Releases](https://github.com/grzegorz2047/TrinityXSX/releases).
2. Wypakuj z archiwum plik `PPSA83601.ffpkg` i skopiuj go przez FTP na PS5 do:
   ```text
   /mnt/ext1/PPSA83601.ffpkg
   ```
3. Pozwól **ShadowMountPlus** zamontować obraz — na ekranie głównym PS5 pojawi się kafelek **TrinityXSX**.
4. Uruchom **TrinityXSX**. Przy pierwszym starcie aplikacja sama przygotuje `/data/homebrew/PPSA83601/` i utworzy `game_paths.txt`.

### Aktualizacja
- **Pełna aktualizacja** (gdy zmienia się `pw_runner.bin`, `win32u.prx`, `Wine Mono` itp.): zamknij TrinityXSX do menu PS5 i podmień `/mnt/ext1/PPSA83601.ffpkg` (lub zaktualizuj z ProsperoStore). Twoje gry, save'y, ustawienia i `xcl.toml` zostają zachowane.
- **Szybka aktualizacja samej biblioteki / XCL:** pobierz `xcl-library-vX.Y.Z.exe` z Releases i podmień przez FTP:
  ```text
  /data/homebrew/PPSA83601/prefix/drive_c/XCL/xcl-library.exe
  ```

---

## 3. Gdzie i jak wgrać swoje gry

### A. Gry Xbox Series X|S oraz już rozpakowane gry PC
Każdą grę wrzuć do **osobnego podfolderu** w jednej z domyślnych lokalizacji:

| Ścieżka na PS5 | Kiedy wybrać |
| --- | --- |
| `/mnt/ext1/xboxseries/<NazwaGry>/` | Dysk zewnętrzny / rozszerzony M.2 (**zalecane**, najwięcej miejsca) |
| `/mnt/usb0/xboxseries/<NazwaGry>/` | Pendrive / dysk USB (`usb0` lub `usb1`) |
| `/data/xboxseries/<NazwaGry>/` | Wewnętrzny dysk SSD PS5 |
| `/data/homebrew/PPSA83601/games/<NazwaGry>/` | Katalog wewnętrzny aplikacji |

- **Xbox Series X|S (`[XSX]`):** folder zawiera `MicrosoftGame.config` (Scarlett) i pliki gry.
- **Rozpakowane gry PC (`[PC64]` / `[PC32]`):** TrinityXSX automatycznie wykrywa rozpakowane foldery gier PC (nawet bez `MicrosoftGame.config`!), rozpoznaje architekturę 32/64-bit, API graficzne (`DXVK` / `vkd3d-proton` / `Zink` dla OpenGL) oraz to, czy gra obsługuje kontroler.
- **Gry z wieloma plikami `.exe`** (np. dodatki lub patche): w bibliotece wciśnij **`R1` -> Informacje o grze -> Plik uruchomieniowy** i wybierz właściwy `.exe` padem.

> **Uwaga:** Poczekaj, aż kopiowanie przez FTP zakończy się w **100%** przed pierwszym uruchomieniem!

### B. Instalacja gier z GOG bezpośrednio na PS5 (bez PC!)
1. Pobierz ze swojego konta GOG pliki **Offline Backup Game Installers** (`setup_<gra>.exe` oraz wszystkie `setup_<gra>-1.bin`, `-2.bin`, ...).
2. Skopiuj je przez FTP na PS5 do katalogu:
   ```text
   /mnt/ext1/xboxseries/installers/
   ```
3. W TrinityXSX naciśnij **`OPTIONS` -> Zainstaluj gry z GOG**, wybierz grę i wciśnij **`X`**.
4. Wbudowany, otwarty `innoextract` rozpakuje grę bezpośrednio na PS5 (z paskiem postępu i ETA), wyciągnie okładkę (`Cover.png`) i doda gotowy wpis do biblioteki. Po instalacji możesz usunąć pliki instalatora przyciskiem **`R1`** (albo włączyć automatyczne usuwanie przyciskiem **`□`** przed instalacją).
5. **Wbudowany Wine Mono `11.3.0`** zapewnia obsługę gier i bibliotek `.NET` / `C++/CLI` / `XNA` od razu po instalacji.

### C. Dodawanie własnych folderów z pada
1. W bibliotece naciśnij **Kwadrat (□)** -> ekran **Foldery gier**.
2. Naciśnij **Kwadrat (□)** (*Dodaj folder*) -> przeglądarka katalogów (startuje w `/mnt`).
3. **Krzyżyk (X)** wchodzi do folderu, **Kółko (O)** cofa poziom wyżej (do `/`), **Trójkąt (Δ)** dodaje bieżący folder.
4. Usunięcie folderu z listy (**Trójkąt** w *Foldery gier*) **nigdy nie kasuje plików gier z dysku**.

---

## 4. Sterowanie DualSense

### W Bibliotece gier
| Przycisk | Działanie |
| --- | --- |
| **D-pad / Lewy analog** | Wybór gry |
| **Krzyżyk (X)** | Uruchom zaznaczoną grę |
| **R1** | Opcje gry (`xcl.toml` dla gier XSX; **Plik uruchomieniowy**, **Sterowanie (PC)** i cache shaderów dla gier PC/GOG) |
| **Kwadrat (□)** | Foldery gier (dodawanie / przeglądanie dysków) |
| **L1** | Web Companion i kod QR (logi, backup/przywracanie save'ów `.zip`, presety, `xcl.toml`) |
| **OPTIONS** | Ustawienia aplikacji (Instalator GOG, język EN/PL/PT/ES, profil offline, Frame Generation 2x/3x/4x, licznik FPS, klawiatura i mysz z PC) |
| **L2** | Dodaj / usuń z ulubionych |
| **R2 / Lewo / Prawo** | Sortowanie (A–Z / Ostatnie / Ulubione) |
| **Trójkąt (Δ)** | Odśwież listę gier |
| **Kółko (O)** | Wyjście do menu PS5 |

### W trakcie gry (Xbox Series i PC / GOG)
| Przycisk DualSense | Działanie w grze |
| --- | --- |
| **`OPTIONS`** | **Xbox `START`** — menu pauzy gry |
| **Lewa połowa Touchpada** (klik po lewej) | **Xbox `BACK` / `Select` / `View`** — mapa, ekwipunek, dziennik, panel główny gry |
| **Prawa połowa Touchpada** (klik po prawej) lub **`L3 + R3`** | Otwórz / zamknij **TRINITYXSX MENU** (FPS/VRAM, **przełącznik `GAMEPAD` ⇄ `MOUSE+KB`**, Restart gry, Zrzut diagnostyki, Powrót do biblioteki, Klawiatura ekranowa, Wyjście do PS5) |
| **Kliknięcie dwoma palcami w Touchpad** | Klawiatura ekranowa PS5 (OSK) |

### Tryb `Mysz i klawiatura (Pad)` dla gier PC/GOG
Możesz go ustawić na stałe per-gra w **`R1 -> Informacje o grze -> Sterowanie (PC)`** (`Auto` automatycznie wybiera ten tryb dla klasycznych gier bez API pada) **albo przełączać na żywo w trakcie gry** w menu pod **prawym Touchpadem** (`INPUT MODE: GAMEPAD ⇄ MOUSE+KB` — przydatne np., gdy gra PC wymaga najpierw kliknięcia myszą w opcjach, żeby włączyć pada):
- **Lewa gałka:** kursor myszy
- **`X` / `R2`:** lewy przycisk myszy (LPM)
- **`□` / `L2`:** prawy przycisk myszy (PPM)
- **`○` / `OPTIONS`:** `Esc` · **`△`:** `Enter`
- **Krzyżak:** `W / A / S / D` · **`L1`:** Spacja · **`R1`:** `Tab`

### Zdalna klawiatura i mysz z komputera / telefonu po LAN
W **`OPTIONS -> Klawiatura i mysz z PC`** możesz włączyć:
- **Przeglądarka (`Web browser`):** otwórz na PC/telefonie adres `http://<IP-PS5>:8361` i wpisz PIN widoczny w menu Touchpada (pełny ekran + `Ctrl+Alt+F12` przechwytuje mysz/klawiaturę, `F9` otwiera menu Touchpada).
- **Program `txin-sender`:** uruchom `txin-sender-vX.Y.Z.exe` na Windows (przekazuje też `Esc` i `Alt+Tab`; przytrzymanie `Esc` przez 1,5 s zawsze wyłącza przechwytywanie).

---

## 5. Pliki konfiguracyjne i zapisy (save'y)

W większości przypadków **nie musisz tworzyć żadnego pliku ręcznie**:
- **Lista folderów z grami:** `/data/homebrew/PPSA83601/game_paths.txt` (edytowana z pada pod **□**).
- **Ustawienia UI i wybory per-gra PC** (język, profile, ulubione, wybrany `.exe` i tryb sterowania PC): `/data/homebrew/PPSA83601/ui_settings.toml`.
- **Konfiguracja gry Xbox Series (`xcl.toml`):** tworzona automatycznie przy pierwszym uruchomieniu gry w:
  ```text
  /data/homebrew/PPSA83601/prefix/drive_c/users/prospero/AppData/Local/XCL/<IdentityName>/xcl.toml
  ```
  Istniejący `xcl.toml` nigdy nie jest nadpisywany. Pełny szablon znajdziesz w [configs/game.example.xcl.toml](../configs/game.example.xcl.toml).
- **Zapisy gier (save'y):**
  - Gry Xbox Series (`XCL`): `/data/homebrew/PPSA83601/prefix/drive_c/users/prospero/AppData/Local/XCL/<IdentityName>/pls/` (wygodny backup `.zip` w **Web Companion `L1`**).
  - Gry PC / GOG: standardowe foldery prefiksu Wine pod `/data/homebrew/PPSA83601/prefix/drive_c/users/prospero/` (`Documents`, `AppData`, `Saved Games`).

---

## 6. Szybkie rozwiązywanie problemów

| Objaw | Co sprawdzić |
| --- | --- |
| **Gry PC nie reagują na gałki pada** | Kliknij **prawą połowę Touchpada** -> przełącz **`INPUT MODE`** na **`MOUSE+KB`** i włącz obsługę pada w opcjach samej gry (albo użyj `R1 -> Sterowanie (PC)` w bibliotece). |
| **Brak przycisku `Back / Select` (mapa / ekwipunek)** | Kliknij **lewą połowę Touchpada** (prawa połowa otwiera menu TrinityXSX, `OPTIONS` to `Start`). |
| **Gra PC ma kilka plików `.exe`** | Zaznacz grę w bibliotece -> wciśnij **`R1`** -> zmień **`Plik uruchomieniowy`** na właściwy `.exe`. |
| **Gry nie widać na liście** | Dla gier XSX sprawdź `MicrosoftGame.config` (`TargetDeviceFamily="Scarlett"`). Dla gier PC sprawdź, czy folder nie jest pusty i wciśnij **Trójkąt (Δ)**, żeby odświeżyć. |
| **Czarny ekran przy pierwszym starcie** | Upewnij się, że kopiowanie FTP skończyło się w 100%, i odczekaj 30–60 s na kompilację shaderów. |
| **Crash / brak pamięci w grze XSX** | W **`R1`** zmień preset na **Low-RAM (4 GB)** lub przełącz **Direct DXIL** na `off (use DXC)`. |

---

## 7. Jak zgłosić grę lub błąd

1. Gdy na PS5 działa FTP, uruchom na komputerze:
   ```sh
   python tools/collect_ps5_logs.py <IP-TWOJEGO-PS5>
   ```
   Skrypt pakuje wyłącznie logi (`startup.log`, `session-*.log`, `xcl.log`, `report.json`) i `xcl.toml` do `trinityxsx-logs-*.zip` — bez plików gier i bez save'ów. (Możesz też pobrać zrzut diagnostyczny jednym kliknięciem z **Web Companion `L1`**.)
2. Otwórz **[Game compatibility report](https://github.com/grzegorz2047/TrinityXSX/issues/new?template=game_compatibility.yml)** na GitHubie i załącz ZIP.
