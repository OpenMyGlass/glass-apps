//! Internationalisation: the four supported locales and the message catalogue.
//!
//! The launcher's UI is multilingual (owner decision, 2026-11-01). The supported languages
//! are **Italian, English, French and Spanish** only — all Latin script, left-to-right, and
//! covered by the single bundled Inter font (Latin-1 and Latin Extended-A, accents and all);
//! there is no RTL and no on-demand font loading.
//!
//! Every user-facing string is a [`Msg`] key with a translation in each [`Locale`]. The
//! catalogue is one table (the [`messages!`] macro), so the four translations of a key sit
//! together and the compiler forces every key to supply all four — a missing translation is
//! a compile error. Keys whose text takes a value carry a `{}` placeholder filled with
//! [`fmt1`] / [`fmt2`]; because each locale positions its own `{}`, word order is free.
//!
//! The current locale lives on [`crate::RenderContext`] (so the card renderer localises
//! footer times) and on the launcher's `App` (which builds every other string). It is read
//! from glassd's settings and chosen at first-boot setup; see the launcher's `settings` and
//! `setup` modules.

/// A supported UI language. Latin script, left-to-right; one font covers all four.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Locale {
    /// Italian.
    It,
    /// English.
    En,
    /// French.
    Fr,
    /// Spanish.
    Es,
}

impl Locale {
    /// Every supported locale, in the first-setup picker's order.
    pub const ALL: [Locale; 4] = [Locale::It, Locale::En, Locale::Fr, Locale::Es];

    /// The locale used before one is chosen and when a tag is not recognised.
    ///
    /// English: a safe international default and the language the committed goldens render
    /// in. The owner's language (Italian, for this device) is set right away by the
    /// first-setup picker — the first screen when no locale is set — or pushed by the
    /// companion, so the default is only ever seen for the instant before setup.
    pub const DEFAULT: Locale = Locale::En;

    /// The language tag (`"it"`, `"en"`, `"fr"`, `"es"`): what glassd stores and the
    /// companion sends.
    pub fn tag(self) -> &'static str {
        match self {
            Locale::It => "it",
            Locale::En => "en",
            Locale::Fr => "fr",
            Locale::Es => "es",
        }
    }

    /// The locale for a language tag, by its primary subtag (`"it"`, `"it-IT"`, `"IT"` all
    /// map to Italian); `None` for a tag of no supported language.
    pub fn from_tag(tag: &str) -> Option<Locale> {
        let primary = tag.split(['-', '_']).next().unwrap_or("");
        Locale::ALL
            .into_iter()
            .find(|l| l.tag().eq_ignore_ascii_case(primary))
    }

    /// Like [`Locale::from_tag`], but falls back to [`Locale::DEFAULT`] for an unknown tag.
    pub fn from_tag_or_default(tag: &str) -> Locale {
        Locale::from_tag(tag).unwrap_or(Locale::DEFAULT)
    }

    /// The language's name in that language (`"Italiano"`, `"English"`, …), for the picker.
    pub fn endonym(self) -> &'static str {
        match self {
            Locale::It => "Italiano",
            Locale::En => "English",
            Locale::Fr => "Français",
            Locale::Es => "Español",
        }
    }
}

impl Default for Locale {
    fn default() -> Locale {
        Locale::DEFAULT
    }
}

/// Fills a template's single `{}` with `a` (for keys like [`Msg::Code`]).
pub fn fmt1(template: &str, a: &str) -> String {
    template.replacen("{}", a, 1)
}

/// Fills a template's two `{}`, left to right, with `a` then `b` (for
/// [`Msg::ConnectedToMore`]). Each locale decides where its placeholders go.
pub fn fmt2(template: &str, a: &str, b: &str) -> String {
    template.replacen("{}", a, 1).replacen("{}", b, 1)
}

/// Defines the [`Msg`] enum and its per-locale table. Every key must give all four
/// translations, so the catalogue can never be partial.
macro_rules! messages {
    ($( $(#[$m:meta])* $key:ident => { it: $it:expr, en: $en:expr, fr: $fr:expr, es: $es:expr } ),* $(,)?) => {
        /// A user-facing message key. Its text in a [`Locale`] comes from [`Msg::get`].
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Msg {
            $( $(#[$m])* $key, )*
        }

        impl Msg {
            /// Every message key (for the completeness test).
            pub const ALL: &'static [Msg] = &[ $( Msg::$key ),* ];

            /// The message in `locale`.
            pub fn get(self, locale: Locale) -> &'static str {
                match self {
                    $( Msg::$key => match locale {
                        Locale::It => $it,
                        Locale::En => $en,
                        Locale::Fr => $fr,
                        Locale::Es => $es,
                    }, )*
                }
            }
        }
    };
}

messages! {
    // ---- relative times (card footers), rendered by glass-ui ----
    JustNow      => { it: "ora",          en: "just now",      fr: "à l'instant",   es: "ahora" },
    MinuteAgo    => { it: "1 minuto fa",  en: "1 minute ago",  fr: "il y a 1 minute",  es: "hace 1 minuto" },
    MinutesAgo   => { it: "{} minuti fa", en: "{} minutes ago", fr: "il y a {} minutes", es: "hace {} minutos" },
    HourAgo      => { it: "1 ora fa",     en: "1 hour ago",    fr: "il y a 1 heure",   es: "hace 1 hora" },
    HoursAgo     => { it: "{} ore fa",    en: "{} hours ago",  fr: "il y a {} heures", es: "hace {} horas" },
    Yesterday    => { it: "ieri",         en: "yesterday",     fr: "hier",          es: "ayer" },
    DaysAgo      => { it: "{} giorni fa", en: "{} days ago",   fr: "il y a {} jours",  es: "hace {} días" },

    // ---- durations (screen-timeout choices) ----
    SecondsN     => { it: "{} secondi",   en: "{} seconds",    fr: "{} secondes",   es: "{} segundos" },
    MinuteOne    => { it: "1 minuto",     en: "1 minute",      fr: "1 minute",      es: "1 minuto" },
    MinutesN     => { it: "{} minuti",    en: "{} minutes",    fr: "{} minutes",    es: "{} minutos" },

    // ---- on/off and radio status ----
    On                 => { it: "Attivo",   en: "On",  fr: "Activé",     es: "Activado" },
    Off                => { it: "Disattivo", en: "Off", fr: "Désactivé",  es: "Desactivado" },
    RadioOnNotConnected => { it: "Attivo, non connesso", en: "On, not connected", fr: "Activé, non connecté", es: "Activado, sin conexión" },
    ConnectedTo        => { it: "Connesso a {}", en: "Connected to {}", fr: "Connecté à {}", es: "Conectado a {}" },
    ConnectedToMore    => { it: "Connesso a {} e altri {}", en: "Connected to {} and {} more", fr: "Connecté à {} et {} autres", es: "Conectado a {} y {} más" },

    WaitingGlassd => { it: "In attesa di glassd", en: "Waiting for glassd", fr: "En attente de glassd", es: "Esperando a glassd" },

    // ---- settings card titles ----
    // Wi-Fi and Bluetooth are product names: the same in every language, but routed through
    // the catalogue so no on-screen title is hardcoded.
    SettingsWifi          => { it: "Wi-Fi", en: "Wi-Fi", fr: "Wi-Fi", es: "Wi-Fi" },
    SettingsBluetooth     => { it: "Bluetooth", en: "Bluetooth", fr: "Bluetooth", es: "Bluetooth" },
    SettingsScreenTimeout => { it: "Timeout schermo", en: "Screen timeout", fr: "Veille de l'écran", es: "Apagado de pantalla" },
    SettingsHeadWake      => { it: "Attiva con la testa", en: "Head wake", fr: "Réveil par la tête", es: "Activar con la cabeza" },
    HeadWakeOnAt          => { it: "Attivo, a {}", en: "On, at {}", fr: "Activé, à {}", es: "Activado, a {}" },
    SettingsWink          => { it: "Strizza l'occhio", en: "Wink", fr: "Clin d'œil", es: "Guiño" },
    SettingsOnHead        => { it: "Rilevamento in testa", en: "On-head detection", fr: "Détection sur la tête", es: "Detección en la cabeza" },
    SettingsNudge         => { it: "Spegni con la testa", en: "Nudge to sleep", fr: "Éteindre avec la tête", es: "Apagar con la cabeza" },
    NudgeHint             => { it: "Un colpetto di testa in su spegne lo schermo", en: "A quick upward flick of the head turns the screen off", fr: "Un petit coup de tête vers le haut éteint l'écran", es: "Un toque rápido de cabeza hacia arriba apaga la pantalla" },
    SettingsDevice        => { it: "Dispositivo", en: "Device", fr: "Appareil", es: "Dispositivo" },
    SettingsLanguage      => { it: "Lingua", en: "Language", fr: "Langue", es: "Idioma" },

    // ---- device card ----
    DeviceLauncherLine     => { it: "glass-launcher {}, renderer {}", en: "glass-launcher {}, renderer {}", fr: "glass-launcher {}, renderer {}", es: "glass-launcher {}, renderer {}" },
    DeviceGlassdLine       => { it: "glassd {}", en: "glassd {}", fr: "glassd {}", es: "glassd {}" },
    DeviceGlassdNotConnected => { it: "glassd non connesso", en: "glassd not connected", fr: "glassd non connecté", es: "glassd sin conexión" },
    DevicePlatform         => { it: "Piattaforma: {}", en: "Platform: {}", fr: "Plateforme : {}", es: "Plataforma: {}" },

    // ---- menu items ----
    TurnOn     => { it: "Attiva",   en: "Turn on",  fr: "Activer",    es: "Activar" },
    TurnOff    => { it: "Disattiva", en: "Turn off", fr: "Désactiver", es: "Desactivar" },
    WakeAngle  => { it: "Angolo di attivazione", en: "Wake angle", fr: "Angle de réveil", es: "Ángulo de activación" },
    Current    => { it: "Attuale",  en: "Current",  fr: "Actuel",     es: "Actual" },
    Delete     => { it: "Elimina",  en: "Delete",   fr: "Supprimer",  es: "Eliminar" },

    // ---- first-boot setup ----
    SetupThisGlass       => { it: "questi Glass", en: "this Glass", fr: "ces Glass", es: "estas Glass" },
    WelcomeTitle         => { it: "Benvenuto", en: "Welcome", fr: "Bienvenue", es: "Bienvenido" },
    WelcomeConnectPhone  => { it: "Colleghiamo il telefono.", en: "Let's connect your phone.", fr: "Connectons votre téléphone.", es: "Conectemos tu teléfono." },
    TapToStart           => { it: "Tocca per iniziare", en: "Tap to start", fr: "Touchez pour commencer", es: "Toca para empezar" },
    PairTitle            => { it: "Associa il telefono", en: "Pair your phone", fr: "Associez votre téléphone", es: "Vincula tu teléfono" },
    PairLine1            => { it: "Nel Bluetooth del telefono", en: "In your phone's Bluetooth", fr: "Dans le Bluetooth du téléphone", es: "En el Bluetooth del teléfono" },
    PairLine2            => { it: "associa con {}", en: "settings, pair with {}", fr: "associez avec {}", es: "vincula con {}" },
    SwipeDownToSkip      => { it: "Scorri giù per saltare", en: "Swipe down to skip", fr: "Balayez vers le bas pour ignorer", es: "Desliza abajo para omitir" },
    PairingTitle         => { it: "Associazione", en: "Pairing", fr: "Association", es: "Vinculación" },
    PairWithThisPhone    => { it: "Associare questo telefono?", en: "Pair with this phone?", fr: "Associer ce téléphone ?", es: "¿Vincular este teléfono?" },
    Code                 => { it: "Codice {}", en: "Code {}", fr: "Code {}", es: "Código {}" },
    TapPairSwipeRefuse   => { it: "Tocca per associare, giù per rifiutare", en: "Tap to pair, swipe down to refuse", fr: "Touchez pour associer, bas pour refuser", es: "Toca para vincular, abajo para rechazar" },
    ConnectedTitle       => { it: "Connesso", en: "Connected", fr: "Connecté", es: "Conectado" },
    ToTarget             => { it: "a {}", en: "to {}", fr: "à {}", es: "a {}" },
    YourPhone            => { it: "il tuo telefono", en: "your phone", fr: "votre téléphone", es: "tu teléfono" },
    TapToFinish          => { it: "Tocca per finire", en: "Tap to finish", fr: "Touchez pour terminer", es: "Toca para terminar" },
    OpenCompanionTitle   => { it: "Apri OpenMyGlass", en: "Open OpenMyGlass", fr: "Ouvrez OpenMyGlass", es: "Abre OpenMyGlass" },
    OpenCompanionLine    => { it: "sul telefono e connetti.", en: "on your phone and connect.", fr: "sur votre téléphone et connectez.", es: "en tu teléfono y conéctate." },
    WaitingGlassApp      => { it: "In attesa dell'app Glass", en: "Waiting for the Glass app", fr: "En attente de l'app Glass", es: "Esperando la app Glass" },
    SkipSetupTitle       => { it: "Saltare la configurazione?", en: "Skip setup?", fr: "Ignorer la configuration ?", es: "¿Omitir configuración?" },
    SkipSetupLine        => { it: "Puoi associare dopo nel Bluetooth.", en: "You can pair later in Bluetooth.", fr: "Vous pourrez associer plus tard dans Bluetooth.", es: "Puedes vincular luego en Bluetooth." },
    TapSkipSwipeBack     => { it: "Tocca per saltare, giù per tornare", en: "Tap to skip, swipe down to go back", fr: "Touchez pour ignorer, bas pour revenir", es: "Toca para omitir, abajo para volver" },
    ChooseLanguage       => { it: "Scegli la lingua", en: "Choose language", fr: "Choisissez la langue", es: "Elige el idioma" },

    // ---- dictation / notes ----
    TakeANote         => { it: "Prendi una nota", en: "Take a note", fr: "Prendre une note", es: "Tomar una nota" },
    TapToSpeak        => { it: "Tocca per parlare", en: "Tap to speak", fr: "Touchez pour parler", es: "Toca para hablar" },
    PhoneNotConnected => { it: "Telefono non connesso", en: "Phone not connected", fr: "Téléphone non connecté", es: "Teléfono no conectado" },
    SpeakNow          => { it: "Parla ora", en: "Speak now", fr: "Parlez", es: "Habla ahora" },
    SourceConnecting  => { it: "Connessione", en: "Connecting", fr: "Connexion", es: "Conectando" },
    SourceListening   => { it: "In ascolto", en: "Listening", fr: "Écoute", es: "Escuchando" },
    SourceProcessing  => { it: "Elaborazione", en: "Processing", fr: "Traitement", es: "Procesando" },
    NoteSource        => { it: "Nota", en: "Note", fr: "Note", es: "Nota" },
    NoteNotTaken      => { it: "Nota non presa", en: "Note not taken", fr: "Note non prise", es: "Nota no tomada" },
    TapToClose        => { it: "Tocca per chiudere", en: "Tap to close", fr: "Touchez pour fermer", es: "Toca para cerrar" },
    DictNothingHeard  => { it: "Non ho sentito nulla", en: "Nothing was heard", fr: "Rien n'a été entendu", es: "No se oyó nada" },
    DictLinkLost      => { it: "Collegamento al telefono perso", en: "The phone link was lost", fr: "La liaison au téléphone est perdue", es: "Se perdió el enlace al teléfono" },
    DictExpired       => { it: "Ci ha messo troppo", en: "That took too long", fr: "Cela a pris trop de temps", es: "Tardó demasiado" },
    DictReplaced      => { it: "È iniziata un'altra nota", en: "Another note was started", fr: "Une autre note a été lancée", es: "Se inició otra nota" },
    DictFailed        => { it: "Dettatura non riuscita", en: "Dictation failed", fr: "Échec de la dictée", es: "Error de dictado" },
    DictPhoneNotConnected => { it: "Il telefono non è connesso", en: "The phone is not connected", fr: "Le téléphone n'est pas connecté", es: "El teléfono no está conectado" },
    DictPhoneCannotNotes  => { it: "L'app del telefono non prende note", en: "The phone app cannot take notes", fr: "L'app du téléphone ne prend pas de notes", es: "La app del teléfono no toma notas" },
    DictNotAvailable      => { it: "Dettatura non disponibile", en: "Dictation is not available", fr: "La dictée n'est pas disponible", es: "El dictado no está disponible" },

    // ---- onboarding welcome card (device timeline) ----
    WelcomeCard => {
        it: "Benvenuto. Scorri avanti per le schede più vecchie, indietro per le impostazioni, giù per tornare.",
        en: "Welcome. Swipe forward for older cards, backward for settings, down to go back.",
        fr: "Bienvenue. Balayez vers l'avant pour les anciennes cartes, l'arrière pour les réglages, le bas pour revenir.",
        es: "Bienvenido. Desliza adelante para tarjetas antiguas, atrás para ajustes, abajo para volver."
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_round_trip_and_fall_back() {
        for l in Locale::ALL {
            assert_eq!(Locale::from_tag(l.tag()), Some(l));
        }
        assert_eq!(Locale::from_tag("it-IT"), Some(Locale::It));
        assert_eq!(Locale::from_tag("EN"), Some(Locale::En));
        assert_eq!(Locale::from_tag("es_ES"), Some(Locale::Es));
        assert_eq!(Locale::from_tag("de"), None);
        assert_eq!(Locale::from_tag_or_default("zz"), Locale::DEFAULT);
    }

    /// No key is missing a translation and none is left blank in any locale (compile-time
    /// exhaustiveness guarantees all four exist; this guards against an empty string).
    #[test]
    fn every_key_has_every_locale() {
        for &m in Msg::ALL {
            for l in Locale::ALL {
                assert!(!m.get(l).is_empty(), "{m:?} is empty in {l:?}");
            }
        }
    }

    /// A key with a `{}` placeholder keeps it in every locale, and a key without one has
    /// none in any locale — so `fmt1`/`fmt2` fill the same number of holes everywhere.
    #[test]
    fn placeholders_are_consistent_across_locales() {
        for &m in Msg::ALL {
            let holes = |l: Locale| m.get(l).matches("{}").count();
            let en = holes(Locale::En);
            for l in Locale::ALL {
                assert_eq!(
                    holes(l),
                    en,
                    "{m:?} has {} placeholders in {l:?} but {en} in English",
                    holes(l)
                );
            }
        }
    }

    /// Catch a copy-paste that left English in a translated slot: a representative key
    /// differs between locales.
    #[test]
    fn translations_are_not_all_english() {
        assert_ne!(
            Msg::WelcomeTitle.get(Locale::It),
            Msg::WelcomeTitle.get(Locale::En)
        );
        assert_ne!(
            Msg::WelcomeTitle.get(Locale::Fr),
            Msg::WelcomeTitle.get(Locale::En)
        );
        assert_ne!(
            Msg::WelcomeTitle.get(Locale::Es),
            Msg::WelcomeTitle.get(Locale::En)
        );
        assert_eq!(Msg::SpeakNow.get(Locale::It), "Parla ora");
    }

    #[test]
    fn formatting_respects_locale_word_order() {
        assert_eq!(fmt1(Msg::Code.get(Locale::It), "0042"), "Codice 0042");
        assert_eq!(
            fmt2(Msg::ConnectedToMore.get(Locale::En), "A", "2"),
            "Connected to A and 2 more"
        );
        assert_eq!(
            fmt2(Msg::ConnectedToMore.get(Locale::It), "A", "2"),
            "Connesso a A e altri 2"
        );
    }
}
