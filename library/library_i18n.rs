//! Lightweight UI translations for TrinityXSX (English, Polish, Portuguese, Spanish).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    Pl,
    Pt,
    Es,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::En, Lang::Pl, Lang::Pt, Lang::Es];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Pl => "pl",
            Lang::Pt => "pt",
            Lang::Es => "es",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Pl => "Polski",
            Lang::Pt => "Português",
            Lang::Es => "Español",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "en" | "english" => Some(Lang::En),
            "pl" | "polski" | "polish" => Some(Lang::Pl),
            "pt" | "portugues" | "português" | "portuguese" => Some(Lang::Pt),
            "es" | "espanol" | "español" | "spanish" => Some(Lang::Es),
            _ => None,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Lang::En => Lang::Pl,
            Lang::Pl => Lang::Pt,
            Lang::Pt => Lang::Es,
            Lang::Es => Lang::En,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Lang::En => Lang::Es,
            Lang::Pl => Lang::En,
            Lang::Pt => Lang::Pl,
            Lang::Es => Lang::Pt,
        }
    }
}

/// Polish pluralization helper (1 -> `one`, 2..4 except 12..14 -> `few`, else `many`).
pub fn plural_pl(n: usize, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    let mod10 = n % 10;
    let mod100 = n % 100;
    if n == 1 {
        one
    } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
        few
    } else {
        many
    }
}

/// Format a count + noun cleanly for the active UI language.
pub fn count_noun(
    lang: Lang,
    n: usize,
    en_one: &str,
    en_many: &str,
    pl_one: &'static str,
    pl_few: &'static str,
    pl_many: &'static str,
) -> String {
    match lang {
        Lang::Pl => format!("{n} {}", plural_pl(n, pl_one, pl_few, pl_many)),
        _ => format!("{n} {}", if n == 1 { en_one } else { en_many }),
    }
}

/// Look up a UI string by its English key. Missing keys fall back to English cleanly.
pub fn tr(lang: Lang, key: &'static str) -> &'static str {
    if lang == Lang::En {
        return key;
    }
    match (lang, key) {
        // Polish
        (Lang::Pl, "Game library") => "Biblioteka gier",
        (Lang::Pl, "games") => "gier",
        (Lang::Pl, "folders") => "folderów",
        (Lang::Pl, "No Xbox Series games found") => "Nie znaleziono gier Xbox Series",
        (Lang::Pl, "Press Square to add game folders (e.g. /mnt/ext1/xboxseries or USB), or copy games into:") => {
            "Naciśnij Kwadrat, aby dodać foldery gier (np. /mnt/ext1/xboxseries lub USB), albo skopiuj gry do:"
        }
        (Lang::Pl, "Press X or Square to open Game folders  ·  Press L1 for Phone/PC Web Companion") => {
            "X / Kwadrat: Foldery gier  ·  L1: Web Companion (QR)  ·  OPTIONS: Profil i język"
        }
        (Lang::Pl, "Config: Custom xcl.toml") => "Konfiguracja: Własny xcl.toml",
        (Lang::Pl, "Config: Tested PS5 preset") => "Konfiguracja: Przetestowany preset PS5",
        (Lang::Pl, "Config: Auto PS5 defaults") => "Konfiguracja: Domyślne ustawienia PS5",
        (Lang::Pl, "Saves: None yet") => "Zapisy: Brak",
        (Lang::Pl, "Press R1 to edit xcl.toml settings") => "Naciśnij R1, aby edytować xcl.toml",
        (Lang::Pl, "X  Play     R1  Settings     Square  Folders     L1  Web/QR     OPTIONS  Profile/Lang     Triangle  Refresh     Circle  Exit") => {
            "X  Graj    R1  Ustawienia    Kwadrat  Foldery    L1  Web/QR    OPTIONS  App/FG    Trójkąt  Odśwież    Kółko  Wyjdź"
        }
        (Lang::Pl, "X  Play    R1  Game    Square  Folders    L1  Web/QR    OPTIONS  Settings    L2  Fav    R2  Sort    Circle  Exit") => {
            "X  Graj    R1  Opcje gry    Kwadrat  Foldery    L1  Web/QR    OPTIONS  Ustawienia    L2  Ulub.    R2  Sortuj    Kółko  Wyjdź"
        }
        (Lang::Pl, "installer(s) ready") => "instalator(y) gotowe",
        (Lang::Pl, "Settings") => "Ustawienia",
        (Lang::Pl, "Install games, adjust display options, player profile and language.") => {
            "Instaluj gry, zmieniaj ustawienia obrazu, profil gracza i język interfejsu."
        }
        (Lang::Pl, "Install GOG Games") => "Zainstaluj gry z GOG",
        (Lang::Pl, "None found") => "Brak instalatorów",
        (Lang::Pl, "ready") => "gotowe",
        (Lang::Pl, "Frame Generation") => "Generowanie klatek",
        (Lang::Pl, "FPS Counter") => "Licznik FPS",
        (Lang::Pl, "PC keyboard & mouse") => "Klawiatura i mysz z PC",
        (Lang::Pl, "Off (USB on the PS5 only)") => "Wyłączone (tylko USB w PS5)",
        (Lang::Pl, "txin-sender program") => "Program txin-sender",
        (Lang::Pl, "Web browser") => "Przeglądarka",
        (Lang::Pl, "Plug a USB keyboard and mouse into the PS5") => "Podłącz klawiaturę i mysz USB do PS5",
        (Lang::Pl, "Run txin-sender.exe on the PC") => "Uruchom txin-sender.exe na komputerze",
        (Lang::Pl, "Open the address shown here or in the Touchpad menu on the PC") => {
            "Otwórz na komputerze adres widoczny tutaj lub w menu Touchpada"
        }
        (Lang::Pl, "On the PC, open") => "Na komputerze otwórz",
        (Lang::Pl, "PS5 address for txin-sender") => "Adres PS5 dla txin-sender",
        (Lang::Pt, "PC keyboard & mouse") => "Teclado e mouse do PC",
        (Lang::Pt, "Off (USB on the PS5 only)") => "Desligado (só USB no PS5)",
        (Lang::Pt, "txin-sender program") => "Programa txin-sender",
        (Lang::Pt, "Web browser") => "Navegador",
        (Lang::Pt, "Plug a USB keyboard and mouse into the PS5") => "Conecte um teclado e mouse USB ao PS5",
        (Lang::Pt, "Run txin-sender.exe on the PC") => "Execute o txin-sender.exe no PC",
        (Lang::Pt, "Open the address shown here or in the Touchpad menu on the PC") => {
            "Abra no PC o endereço mostrado aqui ou no menu do Touchpad"
        }
        (Lang::Pt, "On the PC, open") => "No PC, abra",
        (Lang::Pt, "PS5 address for txin-sender") => "Endereço do PS5 para o txin-sender",
        (Lang::Es, "PC keyboard & mouse") => "Teclado y ratón del PC",
        (Lang::Es, "Off (USB on the PS5 only)") => "Desactivado (solo USB en la PS5)",
        (Lang::Es, "txin-sender program") => "Programa txin-sender",
        (Lang::Es, "Web browser") => "Navegador web",
        (Lang::Es, "Plug a USB keyboard and mouse into the PS5") => "Conecta un teclado y ratón USB a la PS5",
        (Lang::Es, "Run txin-sender.exe on the PC") => "Ejecuta txin-sender.exe en el PC",
        (Lang::Es, "Open the address shown here or in the Touchpad menu on the PC") => {
            "Abre en el PC la dirección que se ve aquí o en el menú del Touchpad"
        }
        (Lang::Es, "On the PC, open") => "En el PC, abre",
        (Lang::Es, "PS5 address for txin-sender") => "Dirección de la PS5 para txin-sender",
        (Lang::Pl, "Frame Generation creates smoother motion (requires restarting TrinityXSX).") => {
            "Generowanie klatek poprawia płynność obrazu (wymaga restartu aplikacji)."
        }
        (Lang::Pl, "Place GOG installer files (setup_*.exe + .bin) in /mnt/ext1/xboxseries/installers/") => {
            "Umieść pliki instalatora GOG (setup_*.exe + .bin) w /mnt/ext1/xboxseries/installers/"
        }
        (Lang::Pl, "Installing") => "Instalowanie",
        (Lang::Pl, "Keep your PS5 awake until installation finishes") => {
            "Nie wyłączaj konsoli PS5 do zakończenia instalacji"
        }
        (Lang::Pl, "No GOG installers found.") => "Nie znaleziono instalatorów GOG.",
        (Lang::Pl, "Copy setup_<game>.exe (+ .bin files) to /mnt/ext1/xboxseries/installers/ and press Triangle to rescan.") => {
            "Skopiuj pliki setup_<gra>.exe (+ pliki .bin) do /mnt/ext1/xboxseries/installers/ i wciśnij Trójkąt, aby odświeżyć."
        }
        (Lang::Pl, "X  Install     Triangle  Rescan     Square  Delete files after:") => {
            "X  Zainstaluj     Trójkąt  Odśwież     Kwadrat  Usuń pliki po instalacji:"
        }
        (Lang::Pl, "Circle  Back") => "Kółko  Wróć",
        (Lang::Pl, "Play") => "Graj",
        (Lang::Pl, "Game") => "Opcje gry",
        (Lang::Pl, "Folders") => "Foldery",
        (Lang::Pl, "Fav") => "Ulub.",
        (Lang::Pl, "Sort") => "Sortuj",
        (Lang::Pl, "Exit") => "Wyjdź",
        (Lang::Pl, "Select") => "Wybierz",
        (Lang::Pl, "Change") => "Zmień",
        (Lang::Pl, "Restart") => "Restart",
        (Lang::Pl, "Back") => "Wróć",
        (Lang::Pl, "Install") => "Zainstaluj",
        (Lang::Pl, "Rescan") => "Odśwież",
        (Lang::Pl, "Delete files after:") => "Usuń po instalacji:",
        (Lang::Pl, "Navigate") => "Wybierz",
        (Lang::Pl, "Change & Save") => "Zmień i zapisz",
        (Lang::Pl, "Reset preset") => "Przywróć preset",
        (Lang::Pl, "Browse") => "Przeglądaj",
        (Lang::Pl, "Add folder") => "Dodaj folder",
        (Lang::Pl, "Remove") => "Usuń z listy",
        (Lang::Pl, "Open") => "Otwórz",
        (Lang::Pl, "Use folder") => "Użyj folderu",
        (Lang::Pl, "Up") => "Wyżej",
        (Lang::Pl, "Cancel") => "Anuluj",
        (Lang::Pl, "Back to library") => "Powrót do biblioteki",
        (Lang::Pl, "Sort: A-Z") => "Sortuj: A-Z",
        (Lang::Pl, "Sort: Recent") => "Sortuj: Ostatnie",
        (Lang::Pl, "Sort: Favorites") => "Sortuj: Ulubione",
        (Lang::Pl, "Added to favorites") => "Dodano do ulubionych",
        (Lang::Pl, "Removed from favorites") => "Usunięto z ulubionych",
        (Lang::Pl, "Game settings") => "Ustawienia gry",
        (Lang::Pl, "D-pad  Navigate     X / Left / Right  Change & Save     Triangle  Reset preset     Circle  Back") => {
            "D-pad  Wybierz     X / Lewo / Prawo  Zmień i zapisz     Trójkąt  Przywróć preset     Kółko  Wróć"
        }
        (Lang::Pl, "Game folders") => "Foldery gier",
        (Lang::Pl, "Add internal, /mnt/ext1 or USB folders. Disconnected folders stay saved.") => {
            "Dodawaj foldery z dysku, /mnt/ext1 lub USB. Odłączone nośniki zostają zapisane."
        }
        (Lang::Pl, "No game folders configured.") => "Brak skonfigurowanych folderów gier.",
        (Lang::Pl, "Press Square to browse and add a folder.") => "Naciśnij Kwadrat, aby wybrać i dodać folder.",
        (Lang::Pl, "Disconnected or unavailable") => "Odłączony lub niedostępny",
        (Lang::Pl, "X  Browse     Square  Add folder     Triangle  Remove     Circle  Back") => {
            "X  Przeglądaj     Kwadrat  Dodaj folder     Trójkąt  Usuń z listy     Kółko  Wróć"
        }
        (Lang::Pl, "Choose folder") => "Wybierz folder",
        (Lang::Pl, "No subfolders here. Press Triangle to add the current folder.") => {
            "Brak podfolderów. Naciśnij Trójkąt, aby dodać bieżący folder."
        }
        (Lang::Pl, "Triangle adds the current folder. Circle goes up one level.") => {
            "Trójkąt dodaje bieżący folder. Kółko przechodzi poziom wyżej."
        }
        (Lang::Pl, "X  Open     Triangle  Use folder     Circle  Up     Square  Cancel") => {
            "X  Otwórz     Trójkąt  Użyj folderu     Kółko  Wyżej     Kwadrat  Anuluj"
        }
        (Lang::Pl, "Phone & PC Web Companion") => "Panel WWW na telefon i komputer",
        (Lang::Pl, "Connect from the same Wi-Fi network to download logs ZIP or edit xcl.toml and folders.") => {
            "Połącz się w tej samej sieci Wi-Fi, aby pobrać paczkę logów ZIP lub edytować xcl.toml i foldery."
        }
        (Lang::Pl, "1. Scan QR code with your phone camera") => "1. Zeskanuj kod QR aparatem w telefonie",
        (Lang::Pl, "Or open this address in any browser:") => "Lub wpisz ten adres w przeglądarce:",
        (Lang::Pl, "What you can do from the browser:") => "Co możesz zrobić z poziomu przeglądarki:",
        (Lang::Pl, "• Download trinityxsx-logs.zip for GitHub bug reports (no FTP needed)") => {
            "• Pobrać trinityxsx-logs.zip do zgłoszenia błędu na GitHubie (bez FTP)"
        }
        (Lang::Pl, "• Edit per-game xcl.toml with syntax validation") => {
            "• Edytować xcl.toml dla każdej gry z walidacją składni"
        }
        (Lang::Pl, "• Edit game_paths.txt, profile and UI language") => {
            "• Edytować foldery gier, profil offline i język interfejsu"
        }
        (Lang::Pl, "Server stops automatically when you launch a game.") => {
            "Serwer wyłącza się automatycznie po uruchomieniu gry."
        }
        (Lang::Pl, "Circle / L1  Back to library") => "Kółko / L1  Powrót do biblioteki",
        (Lang::Pl, "Profile & Language") => "Profil gracza i język",
        (Lang::Pl, "Choose the offline player profile (separate saves) and interface language.") => {
            "Wybierz lokalny profil offline (osobne save'y) oraz język interfejsu TrinityXSX."
        }
        (Lang::Pl, "Interface language") => "Język interfejsu",
        (Lang::Pl, "Active offline profile") => "Aktywny profil offline",
        (Lang::Pl, "Frame Generation (MihawkGen)") => "Generowanie klatek (MihawkGen)",
        (Lang::Pl, "FPS HUD Overlay") => "Licznik FPS na ekranie (HUD)",
        (Lang::Pl, "On") => "Włączone",
        (Lang::Pl, "Off") => "Wyłączone",
        (Lang::Pl, "All installed") => "Wszystkie zainstalowane",
        (Lang::Pl, "Installed") => "Zainstalowano",
        (Lang::Pl, "Delete installer") => "Usuń instalator",
        (Lang::Pl, "Installation in progress — buttons locked until finished") => {
            "Trwa instalacja — przyciski zablokowane do zakończenia"
        }
        (Lang::Pl, "Press Circle to return and play") => "Naciśnij Kółko, aby wrócić i zagrać",
        (Lang::Pl, "Install failed") => "Instalacja nieudana",
        (Lang::Pl, "Removed installer files") => "Usunięto pliki instalatora",
        (Lang::Pl, "Delete installer after install") => "Usuwaj instalator po instalacji",
        (Lang::Pl, "estimating time...") => "szacowanie czasu...",
        (Lang::Pl, "left") => "pozostało",
        (Lang::Pl, "Available") => "Dostępny",
        (Lang::Pl, "Not found") => "Nie znaleziono",
        (Lang::Pl, "Saves") => "Zapisy",
        (Lang::Pl, "Folder") => "Folder",
        (Lang::Pl, "Web & Logs") => "Panel WWW i logi",
        (Lang::Pl, "Removing a folder never deletes games on disk.") => {
            "Usunięcie folderu nigdy nie kasuje gier z dysku."
        }
        (Lang::Pl, "Xbox Series game") => "Gra Xbox Series",
        (Lang::Pl, "Restart now") => "Uruchom ponownie",
        (Lang::Pl, "restart") => "wymaga restartu",
        (Lang::Pl, "Config: PC/GOG native mode (auto)") => "Konfiguracja: Tryb PC/GOG (automatyczny)",
        (Lang::Pl, "Press R1 for game info & shader cache") => "Naciśnij R1: informacje o grze i cache",
        (Lang::Pl, "Launch mode") => "Tryb uruchomienia",
        (Lang::Pl, "Compatibility status") => "Status kompatybilności",
        (Lang::Pl, "Automatic PC/GOG profile") => "Automatyczny profil PC/GOG",
        (Lang::Pl, "PC/GOG games run directly under Wine/DXVK without Xbox xcl.toml settings.") => {
            "Gry PC/GOG działają bezpośrednio przez Wine/DXVK (nie używają ustawień Xbox xcl.toml)."
        }
        (Lang::Pl, "Compatibility preset") => "Preset kompatybilności",
        (Lang::Pl, "Console model") => "Model konsoli",
        (Lang::Pl, "Title memory budget") => "Limit pamięci gry",
        (Lang::Pl, "Coherent UMA window") => "Okno pamięci UMA",
        (Lang::Pl, "Direct DXIL translator") => "Bezpośredni translator DXIL",
        (Lang::Pl, "Raytracing (DXR)") => "Śledzenie promieni (DXR)",
        (Lang::Pl, "Network access") => "Dostęp do sieci",
        (Lang::Pl, "Xbox Live placeholder tokens") => "Zastępcze tokeny Xbox Live",
        (Lang::Pl, "Log verbosity") => "Szczegółowość logów",
        (Lang::Pl, "Diagnostic tracing") => "Śledzenie diagnostyczne",
        (Lang::Pl, "Stub nvngx_dlss.dll") => "Zaślepka nvngx_dlss.dll",
        (Lang::Pl, "D3D12 Feature Level (vkd3d)") => "Poziom funkcji D3D12 (vkd3d)",
        (Lang::Pl, "Offline (false)") => "Offline (wył.)",
        (Lang::Pl, "Online (true)") => "Online (wł.)",
        (Lang::Pl, "Disabled (false)") => "Wyłączone",
        (Lang::Pl, "Enabled (true)") => "Włączone",
        (Lang::Pl, "Auto (console default)") => "Auto (domyślny)",
        (Lang::Pl, "Auto (GPU default)") => "Auto (domyślny GPU)",
        (Lang::Pl, "all (fastest on PS5)") => "wszystkie (najszybsze na PS5)",
        (Lang::Pl, "1 (partial)") => "1 (częściowy)",
        (Lang::Pl, "off (use DXC)") => "wyłączony (użyj DXC)",
        (Lang::Pl, "On (activity + net + live flush)") => "Włączone (pełne logi na żywo)",
        (Lang::Pl, "Off (normal play)") => "Wyłączone (normalna gra)",
        (Lang::Pl, "Clear") => "Wyczyść",
        (Lang::Pl, "Cleared shader cache") => "Wyczyszczono cache shaderów",
        (Lang::Pl, "Install DRM-free GOG offline installers stored in /mnt/ext1/xboxseries/installers/.") => {
            "Instaluj gry GOG z instalatorów offline w /mnt/ext1/xboxseries/installers/."
        }
        (Lang::Pl, "Change the display language of the TrinityXSX menu immediately.") => {
            "Zmienia język interfejsu TrinityXSX natychmiast, bez restartu."
        }
        (Lang::Pl, "Switch offline player profile (separate saves per profile). Add profiles via Web Companion (L1).") => {
            "Przełącza lokalny profil gracza (osobne zapisy). Nowe profile dodasz w panelu WWW (L1)."
        }
        (Lang::Pl, "Show or hide the live FPS counter in the top-left corner (requires restart).") => {
            "Pokazuje lub ukrywa licznik FPS w lewym górnym rogu ekranu (wymaga restartu)."
        }
        (Lang::Pl, "Control games with a PC keyboard and mouse over LAN (Browser with PIN or txin-sender).") => {
            "Steruj grami klawiaturą i myszą z PC po sieci LAN (Przeglądarka z PIN-em lub txin-sender)."
        }
        (Lang::Pl, "Restart TrinityXSX cleanly to apply display changes such as Frame Generation or FPS counter.") => {
            "Restartuje TrinityXSX, aby zastosować zmiany wyświetlania (generowanie klatek, licznik FPS)."
        }
        (Lang::Pl, "Deletes the compiled shaders of ALL games (XCL, vkd3d-proton, RADV); they rebuild on next launch. Saves and xcl.toml stay. Press X.") => {
            "Usuwa skompilowane shadery WSZYSTKICH gier (odbudują się przy następnym uruchomieniu). Zapisy zostają nietknięte."
        }
        (Lang::Pl, "Rescanned folders") => "Odświeżono foldery",
        (Lang::Pl, "Saved settings") => "Zapisano ustawienia",
        (Lang::Pl, "Applied preset") => "Zastosowano preset",
        (Lang::Pl, "Game info") => "Informacje o grze",
        (Lang::Pl, "Info") => "Opcje",
        (Lang::Pl, "Series S uses less RAM and safer LOD on PS5. Switch to Series X only if a game refuses Series S.") => {
            "Series S zużywa mniej RAM na PS5. Zmień na Series X tylko, gdy gra odrzuca Series S."
        }
        (Lang::Pl, "6144 MiB is recommended on PS5. Lower to 4096 MiB if a game crashes with out-of-memory errors.") => {
            "Zalecane 6144 MiB na PS5. Obniż do 4096 MiB, jeśli gra wywala się przez brak pamięci."
        }
        (Lang::Pl, "CPU/GPU mapped texture window. Raise to 1536 or 2048 MiB if xcl.log reports coherent window exhaustion.") => {
            "Okno współdzielonej pamięci CPU/GPU. Zwiększ do 1536 lub 2048 MiB przy błędach pamięci tekstur."
        }
        (Lang::Pl, "\"all\" skips heavy multi-threaded DXC shader compilation on PS5. Switch to \"off\" if shaders look broken.") => {
            "\"wszystkie\" pomija ciężką kompilację DXC na PS5. Wyłącz tylko, gdy shadery wyglądają źle."
        }
        (Lang::Pl, "Keep \"off\" while testing a new game; enable tier1_0/tier1_1 only after it runs stably.") => {
            "Zostaw wyłączone podczas testów; włącz tier1_0/tier1_1 dopiero gdy gra działa stabilnie."
        }
        (Lang::Pl, "Offline blocks external sockets so games boot cleanly without hanging on Xbox Live.") => {
            "Tryb offline blokuje sieć, dzięki czemu gry uruchamiają się bez zawieszania na Xbox Live."
        }
        (Lang::Pl, "Keep Disabled for offline play — placeholder tokens can cause infinite sign-in loops.") => {
            "Zostaw wyłączone w trybie offline — zastępcze tokeny mogą powodować pętlę logowania."
        }
        (Lang::Pl, "Use \"info\" for normal play. Switch to \"debug\" when capturing logs for a GitHub bug report.") => {
            "Używaj \"info\" do gry. Przełącz na \"debug\" przy zbieraniu logów do zgłoszenia błędu."
        }
        (Lang::Pl, "Flushes xcl.log immediately and records memory/activity every 2s. Helpful when diagnosing hangs.") => {
            "Zapisuje xcl.log na bieżąco i loguje pamięć co 2 s. Pomocne przy diagnozowaniu zawieszeń."
        }
        (Lang::Pl, "Replaces NVIDIA DLSS DLL imports with a harmless stub on PS5's AMD RDNA2 GPU.") => {
            "Zastępuje biblioteki NVIDIA DLSS bezpieczną zaślepką na układzie AMD RDNA2 w PS5."
        }
        (Lang::Pl, "Sets VKD3D_FEATURE_LEVEL for vkd3d-proton. Try 12_0 if a game fails D3D12CreateDevice on PS5 RADV.") => {
            "Wymusza poziom funkcji D3D12 (vkd3d-proton). Spróbuj 12_0 przy błędzie inicjalizacji grafiki."
        }
        (Lang::Pl, "Restored recommended PS5 preset") => "Przywrócono zalecany preset PS5",
        (Lang::Pl, "MihawkGen neural interpolation (Vulkan / DXVK / D3D12). Restart TrinityXSX to apply.") => {
            "Interpolacja klatek MihawkGen (Vulkan / DXVK / D3D12). Zrestartuj TrinityXSX po zmianie."
        }
        (Lang::Pl, "Restart TrinityXSX to apply") => "Zrestartuj TrinityXSX, aby zastosować",
        (Lang::Pl, "Restart TrinityXSX now") => "Zrestartuj TrinityXSX teraz",
        (Lang::Pl, "Apply FrameGen") => "Zastosuj FrameGen",
        (Lang::Pl, "Apply changes") => "Zastosuj zmiany",
        (Lang::Pl, "Press X") => "Naciśnij X",
        (Lang::Pl, "Clear Shader Cache") => "Wyczyść cache shaderów",
        (Lang::Pl, "Empty") => "Pusty",
        (Lang::Pl, "Custom gamertags can also be added from the Phone/PC Web Companion (L1).") => {
            "Własne nazwy profili (gamertagi) możesz też dodać w panelu WWW na telefonie (L1)."
        }
        (Lang::Pl, "X / Left / Right  Change & Save     Triangle  Restart TrinityXSX     Circle  Back") => {
            "X / Lewo / Prawo  Zmień i zapisz     Trójkąt  Restart TrinityXSX     Kółko  Wróć"
        }
        (Lang::Pl, "X / Left / Right  Change & Save     Circle  Back") => {
            "X / Lewo / Prawo  Zmień i zapisz     Kółko  Wróć"
        }
        (Lang::Pl, "Library updated automatically") => "Biblioteka odświeżona automatycznie",
        (Lang::Pl, "PC Controls") => "Sterowanie (PC)",
        (Lang::Pl, "Executable") => "Plik uruchomieniowy",
        (Lang::Pl, "Choose which .exe to launch when a PC game folder contains multiple executables (e.g. expansions or patches).") => {
            "Wybierz plik .exe do uruchomienia, jeśli folder gry PC ma kilka plików (np. dodatki lub patche)."
        }
        (Lang::Pl, "Auto") => "Auto",
        (Lang::Pl, "Gamepad") => "Pad",
        (Lang::Pl, "Mouse & KB") => "Mysz + KB",
        (Lang::Pl, "Gamepad (XInput)") => "Pad (XInput)",
        (Lang::Pl, "Mouse & Keyboard (Pad)") => "Mysz i klawiatura (Pad)",
        (Lang::Pl, "Auto uses Gamepad if the game supports controllers, or Stick-as-Mouse otherwise. You can also switch live via the Touchpad menu.") => {
            "Auto wybiera Pad, jeśli gra obsługuje kontroler, lub Mysz na gałce. W grze przełączysz tryb w menu Touchpada."
        }

        // Portuguese
        (Lang::Pt, "PC Controls") => "Controles (PC)",
        (Lang::Pt, "Executable") => "Executável",
        (Lang::Pt, "Gamepad") => "Controle",
        (Lang::Pt, "Mouse & KB") => "Mouse + Teclado",
        (Lang::Pt, "Gamepad (XInput)") => "Controle (XInput)",
        (Lang::Pt, "Mouse & Keyboard (Pad)") => "Mouse e teclado (Controle)",
        (Lang::Pt, "Game library") => "Biblioteca de jogos",
        (Lang::Pt, "games") => "jogos",
        (Lang::Pt, "folders") => "pastas",
        (Lang::Pt, "No Xbox Series games found") => "Nenhum jogo Xbox Series encontrado",
        (Lang::Pt, "Game settings") => "Configurações do jogo",
        (Lang::Pt, "Game folders") => "Pastas de jogos",
        (Lang::Pt, "Choose folder") => "Escolher pasta",
        (Lang::Pt, "Disconnected or unavailable") => "Desconectada ou indisponível",
        (Lang::Pt, "Phone & PC Web Companion") => "Configurações pelo celular ou PC",
        (Lang::Pt, "Profile & Language") => "Perfil e Idioma",
        (Lang::Pt, "Interface language") => "Idioma da interface",
        (Lang::Pt, "Active offline profile") => "Perfil offline ativo",
        (Lang::Pt, "Library updated automatically") => "Lista de jogos atualizada automaticamente",

        // Spanish
        (Lang::Es, "PC Controls") => "Controles (PC)",
        (Lang::Es, "Executable") => "Ejecutable",
        (Lang::Es, "Gamepad") => "Mando",
        (Lang::Es, "Mouse & KB") => "Ratón + Teclado",
        (Lang::Es, "Gamepad (XInput)") => "Mando (XInput)",
        (Lang::Es, "Mouse & Keyboard (Pad)") => "Ratón y teclado (Mando)",
        (Lang::Es, "Game library") => "Biblioteca de juegos",
        (Lang::Es, "games") => "juegos",
        (Lang::Es, "folders") => "carpetas",
        (Lang::Es, "No Xbox Series games found") => "No se encontraron juegos de Xbox Series",
        (Lang::Es, "Game settings") => "Ajustes del juego",
        (Lang::Es, "Game folders") => "Carpetas de juegos",
        (Lang::Es, "Choose folder") => "Elegir carpeta",
        (Lang::Es, "Disconnected or unavailable") => "Desconectada o no disponible",
        (Lang::Es, "Phone & PC Web Companion") => "Panel Web para móvil y PC",
        (Lang::Es, "Profile & Language") => "Perfil e Idioma",
        (Lang::Es, "Interface language") => "Idioma de la interfaz",
        (Lang::Es, "Active offline profile") => "Perfil offline activo",
        (Lang::Es, "Library updated automatically") => "Biblioteca actualizada automáticamente",

        _ => key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cycles_and_falls_back_to_english() {
        assert_eq!(Lang::parse("pl"), Some(Lang::Pl));
        assert_eq!(Lang::parse("PT"), Some(Lang::Pt));
        assert_eq!(Lang::En.next(), Lang::Pl);
        assert_eq!(Lang::Es.next(), Lang::En);
        assert_eq!(tr(Lang::Pl, "Game folders"), "Foldery gier");
        assert_eq!(tr(Lang::Pt, "Untranslated key"), "Untranslated key");
    }

    #[test]
    fn polish_pluralization_handles_teens_and_twenties() {
        assert_eq!(plural_pl(0, "gra", "gry", "gier"), "gier");
        assert_eq!(plural_pl(1, "gra", "gry", "gier"), "gra");
        assert_eq!(plural_pl(2, "gra", "gry", "gier"), "gry");
        assert_eq!(plural_pl(4, "gra", "gry", "gier"), "gry");
        assert_eq!(plural_pl(5, "gra", "gry", "gier"), "gier");
        assert_eq!(plural_pl(12, "gra", "gry", "gier"), "gier");
        assert_eq!(plural_pl(14, "gra", "gry", "gier"), "gier");
        assert_eq!(plural_pl(22, "gra", "gry", "gier"), "gry");
        assert_eq!(plural_pl(25, "gra", "gry", "gier"), "gier");
        assert_eq!(count_noun(Lang::Pl, 1, "file", "files", "plik", "pliki", "plików"), "1 plik");
        assert_eq!(count_noun(Lang::Pl, 3, "file", "files", "plik", "pliki", "plików"), "3 pliki");
        assert_eq!(count_noun(Lang::En, 1, "file", "files", "plik", "pliki", "plików"), "1 file");
        assert_eq!(count_noun(Lang::En, 3, "file", "files", "plik", "pliki", "plików"), "3 files");
    }
}
