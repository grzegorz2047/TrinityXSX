# Poradnik użytkownika TrinityXSX (PS5)

Prosta instrukcja krok po kroku: jak zainstalować **TrinityXSX (`PPSA83601`)**, gdzie i jak wgrać własną grę z Xbox Series X|S, które pliki konfiguracyjne mają znaczenie oraz jak zgłaszać błędy.

> **Ważne:** TrinityXSX **nie zawiera żadnych gier, kluczy, BIOS-u ani plików SDK Sony**. Musisz użyć własnej, rozpakowanej kopii gry z Xbox Series X|S (GDK / Scarlett).

---

## 1. Czego potrzebujesz

- PS5 z działającym homebrew (**ShadowMountPlus** oraz **elfldr**, np. etaHEN + kstuff).
- Klienta FTP na komputerze (FileZilla, WinSCP lub skrypt `tools/ftp_put_tree.py`).
- Dysku zewnętrznego/rozszerzonego pod `/mnt/ext1` (lub pamięci wewnętrznej `/data` / pendrive'a `/mnt/usb0`).
- Własnej, **rozpakowanej** gry Xbox Series X|S zawierającej plik `MicrosoftGame.config` oraz wskazany w nim plik `.exe`.

---

## 2. Instalacja TrinityXSX na PS5

1. Pobierz najnowszy plik **`PPSA83601-v0.1.0.ffpkg.zip`** ze strony [GitHub Releases](https://github.com/grzegorz2047/TrinityXSX/releases).
2. Wypakuj z archiwum plik `PPSA83601.ffpkg`.
3. Skopiuj `PPSA83601.ffpkg` przez FTP na PS5 do:
   ```text
   /mnt/ext1/PPSA83601.ffpkg
   ```
4. Pozwól **ShadowMountPlus** zamontować obraz — na ekranie głównym PS5 pojawi się kafelek **TrinityXSX**.
5. Uruchom **TrinityXSX**. Przy pierwszym starcie aplikacja sama przygotuje katalog `/data/homebrew/PPSA83601/` i utworzy `game_paths.txt`.

### Szybka aktualizacja w przyszłości
Jeśli nowa wersja aktualizuje tylko bibliotekę / XCL, nie musisz wgrywać od nowa całego obrazu `.ffpkg`:
- Pobierz `xcl-library-vX.Y.Z.exe` z Releases i podmień przez FTP plik:
  ```text
  /data/homebrew/PPSA83601/prefix/drive_c/XCL/xcl-library.exe
  ```

---

## 3. Gdzie i jak wgrać swoją grę

Każdą grę wrzuć do **osobnego podfolderu** w jednej z domyślnych lokalizacji:

| Ścieżka na PS5 | Kiedy wybrać |
| --- | --- |
| `/mnt/ext1/xboxseries/<NazwaGry>/` | Dysk zewnętrzny / rozszerzony (**zalecane**, najwięcej miejsca) |
| `/mnt/usb0/xboxseries/<NazwaGry>/` | Pendrive / dysk USB (`usb0` lub `usb1`) |
| `/data/xboxseries/<NazwaGry>/` | Wewnętrzny dysk SSD PS5 |
| `/data/homebrew/PPSA83601/games/<NazwaGry>/` | Katalog wewnętrzny aplikacji |

### Wymagana struktura folderu gry
W środku folderu muszą znaleźć się rozpakowane pliki gry, w szczególności `MicrosoftGame.config` oraz plik `.exe` wersji **Scarlett** (Xbox Series X|S):

```text
/mnt/ext1/xboxseries/
  MojaGra/
    MicrosoftGame.config
    MojaGra.exe
    ...wszystkie pliki DLL i podfoldery z danymi gry...
```

> **Uwaga:** Poczekaj, aż kopiowanie przez FTP zakończy się w **100%** przed pierwszym uruchomieniem! Jeśli odpalisz grę w trakcie kopiowania, często skończy się to czarnym ekranem (bo brakuje jeszcze plików `data/`).

### Dodawanie własnych folderów z pada (jak w TrinityX360)
Możesz wskazać dowolny folder na PS5 bezpośrednio w aplikacji:
1. W bibliotece naciśnij **Kwadrat (□)** -> otwiera ekran **Game folders**.
2. Naciśnij **Kwadrat (□)** (*Add folder*), żeby otworzyć przeglądarkę katalogów (startuje w `/mnt`).
3. Poruszaj się **D-padem lub lewym analogiem**:
   - **Krzyżyk (X)** — wejście do podfolderu (foldery z grą mają znacznik `[Xbox Series game]`)
   - **Kółko (O)** — poziom wyżej (w `/mnt` przechodzi do `/`, skąd widać `/data`, `/mnt/usb0`, `/mnt/ext1` itd.)
   - **Trójkąt (Δ)** — dodanie aktualnie otwartego folderu do biblioteki
   - **Kwadrat (□)** — anuluj
4. Usunięcie folderu (**Trójkąt** w *Game folders*) usuwa go tylko z listy skanowania — **nigdy nie kasuje plików gier z dysku**. Odłączone dyski USB zostają zapisane na liście.

---

## 4. Sterowanie DualSense

| Przycisk | Biblioteka gier | Game folders | Przeglądarka folderów |
| --- | --- | --- | --- |
| **D-pad / Lewy analog** | Wybór gry | Wybór folderu | Wybór podfolderu |
| **Krzyżyk (X)** | Uruchom grę | Przeglądaj zaznaczony folder | Wejdź do podfolderu |
| **Kwadrat (□)** | Zarządzaj folderami | Dodaj nowy folder | Anuluj |
| **Trójkąt (Δ)** | Odśwież listę gier | Usuń folder z listy | Użyj bieżącego folderu |
| **Kółko (O)** | Wyjście do menu PS5 | Powrót do biblioteki | Poziom wyżej |

---

## 5. Które pliki konfiguracyjne są potrzebne?

W większości przypadków **nie musisz tworzyć żadnego pliku ręcznie**. Dla użytkownika liczą się wyłącznie dwa pliki na PS5:

### 1. `game_paths.txt` — lista folderów z grami
- Ścieżka: `/data/homebrew/PPSA83601/game_paths.txt`
- Tworzy się automatycznie i można go wygodnie edytować z pada (**Kwadrat**).
- Jedna ścieżka PS5 w linii (np. `/mnt/ext1/xboxseries`).

### 2. `xcl.toml` — ustawienia konkretnej gry (tworzony automatycznie!)
- Ścieżka:
  ```text
  /data/homebrew/PPSA83601/prefix/drive_c/users/prospero/AppData/Local/XCL/<IdentityName>/xcl.toml
  ```
  (gdzie `<IdentityName>` to pole `Identity Name` z `MicrosoftGame.config` danej gry).
- Przy pierwszym uruchomieniu gry TrinityXSX samo tworzy bezpieczny profil offline dla PS5 (`device_type = "lockhart"`, `title_memory_mb = 6144`, `coherent_mb = 1024`, `raytracing = "off"`, `direct_dxil = "all"`, `network = false`).
- **Istniejący plik `xcl.toml` nigdy nie jest nadpisywany** — Twoje zmiany zostają na stałe.
- Pełny opisany szablon znajdziesz w [configs/game.example.xcl.toml](../configs/game.example.xcl.toml).

#### Najważniejsze opcje w `xcl.toml`, gdy jakaś gra nie chce wstać

| Opcja | Domyślnie | Kiedy zmienić |
| --- | --- | --- |
| `device_type` / `console_type_raw` | `"lockhart"` / `5` (Series S) | Zmień na `"anaconda"` / `6` (Series X) tylko jeśli gra wymaga profilu Series X. |
| `title_memory_mb` | `6144` | Obniż do `4096`, jeśli gra wywala się z błędem braku pamięci (`dmem` / OOM). |
| `coherent_mb` | `1024` | Zwiększ do `1536`, jeśli w `xcl.log` pojawia się wyczerpanie okna coherent UMA. |
| `direct_dxil` | `"all"` | Ustaw `"off"`, jeśli gra ma błędy graficzne/shaderów (przełącza na kompilację przez DXC). |
| `xbox_live_tokens` / `network` | `false` / `false` | Zostaw `false` do gry offline — zapobiega zapętleniu logowania do Xbox Live. |
| `raytracing` | `"off"` | Zostaw `"off"` na start. |
| `[aliases]` | *(puste)* | Pozwala zaślepić nieobsługiwane biblioteki PC/NVIDIA, np. `"nvngx_dlss.dll" = "@stub"`. |

### 3. Gdzie są zapisy gier (save'y)?
```text
/data/homebrew/PPSA83601/prefix/drive_c/users/prospero/AppData/Local/XCL/<IdentityName>/pls/
```
Żeby zrobić kopię zapasową save'ów, wystarczy skopiować ten katalog przez FTP.

---

## 6. Szybkie rozwiązywanie problemów

| Objaw | Co sprawdzić |
| --- | --- |
| **Gry nie widać na liście** | Sprawdź, czy w folderze gry jest `MicrosoftGame.config` z wpisem `TargetDeviceFamily="Scarlett"` i odpowiadający plik `.exe`. Naciśnij **Trójkąt**, żeby przeskanować ponownie. |
| **Czarny ekran po uruchomieniu** | Upewnij się, że gra skopiowała się w 100% (wszystkie pliki `data/`). Przy pierwszym starcie odczekaj 30–60 s na budowanie cache shaderów. Zajrzyj do `xcl.log`. |
| **Crash / powrót do menu PS5** | Sprawdź `startup.log` i `xcl.log`. Spróbuj obniżyć `title_memory_mb = 4096` albo ustawić `direct_dxil = "off"`. |
| **Zapętlone logowanie Xbox Live** | Upewnij się, że w `xcl.toml` gry masz `xbox_live_tokens = false` i `network = false`. |
| **Pad nie działa w grze** | Sprawdź, czy w `xcl.toml` masz `keyboard_mouse = false`. |

---

## 7. Jak zgłosić błąd gry, żebyśmy mogli go wspólnie naprawić

1. Na komputerze (gdy na PS5 działa serwer FTP) uruchom narzędzie do zebrania logów:
   ```sh
   python tools/collect_ps5_logs.py <IP-TWOJEGO-PS5>
   ```
   Skrypt zbiera **wyłącznie logi i `xcl.toml`** (`startup.log`, `session-*.log`, `vkd3d-trinityxsx-library.log`, `xcl.log`, `report.json`) do jednego pliku `trinityxsx-logs-*.zip` — bez żadnych plików gier ani save'ów.
2. Otwórz formularz **[Game compatibility report](https://github.com/grzegorz2047/TrinityXSX/issues/new?template=game_compatibility.yml)** na GitHubie i załącz wygenerowany plik ZIP.
