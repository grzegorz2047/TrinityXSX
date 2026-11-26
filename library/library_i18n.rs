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
        (Lang::Pl, "On") => "Włączony",
        (Lang::Pl, "Off") => "Wyłączone",
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

        // Portuguese
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
}
