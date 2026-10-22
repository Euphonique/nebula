# Nebula

Schneller, nativer Code-Editor für Windows, inspiriert von Panic Nova. In Rust geschrieben. Schwerpunkt ist das direkte Bearbeiten von Websites auf Servern per FTP, FTPS und SFTP.

## Bauen und starten

Voraussetzungen: Rust (stable, MSVC-Toolchain) und die Visual Studio Build Tools (für die tree-sitter-Grammatiken in C).

```powershell
cargo run --release                     # starten
cargo run --release -- C:\pfad\projekt  # Ordner direkt öffnen
cargo test                              # Unit- und Integrationstests (inkl. FTP/SFTP)
```

Das Ergebnis ist eine einzelne Datei ohne weitere Abhängigkeiten: `target\release\nebula.exe`.

## Funktionen

**Server und Dateien**
- Server-Verwaltung für SFTP, FTP und FTPS (explizites TLS über Windows SChannel)
- Server in einklappbaren Gruppen ordnen (Feld „Gruppe“ im Serverdialog oder Rechtsklick → „Gruppe“; Gruppen per Rechtsklick umbenennen oder auflösen), Server duplizieren
- Import aus FileZilla (`sitemanager.xml`, Ordner werden zu Gruppen, Passwörter auf Wunsch) und aus Nebula-Exporten; Export der Serverliste als JSON ohne Passwörter. Bereits vorhandene Server werden erkannt und nicht doppelt angelegt (≡-Menü im Reiter „Server“, Menü „Server“ oder Befehlspalette)
- Terminal im Panel unter dem Editor (Strg+Ö oder Knopf in der Titelleiste): lokale Shells (PowerShell, Windows PowerShell, Git Bash, Eingabeaufforderung) im Projektordner, z. B. für Claude Code oder npm; Standard-Shell in den Einstellungen wählbar. SSH-Terminals über das +-Menü, das Kontextmenü eines Servers oder die Befehlspalette: mehrere Sitzungen als Tabs, 256 Farben, Scrollback per Mausrad, Markieren und Kopieren, Einfügen, Neuverbinden mit Enter. Nutzt Zugangsdaten und Host-Key-Prüfung der Server (bei FTP-Servern Port 22) und wechselt in den Startpfad.
- Gespeicherte Serverbefehle (z. B. Cache leeren, Deploy): im SSH-Terminal über „Befehle ▾“ anlegen und ausführen, außerdem per Rechtsklick auf den Server („Befehl ausführen“) und in der Befehlspalette; öffnet bei Bedarf ein Terminal und führt den Befehl im Startordner aus
- Berechtigungen (chmod) für Dateien und Ordner auf dem Server, auf Wunsch rekursiv (nur Dateien, nur Ordner oder alles); die aktuellen Rechte stehen im Tooltip
- Dateien auf dem Server öffnen und mit Strg+S direkt hochladen
- Konflikterkennung: Wurde eine Datei auf dem Server inzwischen geändert, fragt Nebula nach (Überschreiben, Server-Version laden oder Abbrechen) und zeigt auf Wunsch die Unterschiede an
- Kopieren, Ausschneiden, Einfügen, Duplizieren, Umbenennen und Löschen, lokal und auf Servern, auch mit Mehrfachauswahl (Strg+Klick)
- Drag & Drop im Baum: verschiebt, mit Strg wird kopiert. Ordner öffnen sich beim Darüberhalten. Lokale Dateien lassen sich aus dem Fenster heraus in den Explorer (oder andere Programme) ziehen.
- Dateien aus dem Explorer in einen Ordner ziehen oder dort einfügen: lokal kopieren oder auf den Server hochladen. In Nebula kopierte Dateien lassen sich im Explorer einfügen.
- Zwischen lokal und Server sowie zwischen zwei Servern wird automatisch hoch- bzw. heruntergeladen, Ordner rekursiv. Bei Namenskonflikten fragt ein Dialog: Ersetzen, Beide behalten oder Überspringen.
- Veröffentlichen: Ein lokaler Projektordner wird mit einem Server-Ordner verknüpft. Hochgeladen werden nur geänderte Dateien, auf Wunsch automatisch beim Speichern.
- Passwörter liegen in der Windows-Anmeldeinformationsverwaltung. SFTP-Host-Keys werden beim ersten Verbinden bestätigt und danach geprüft.

**Editor**
- 26 Sprachen mit tree-sitter-Highlighting: HTML, CSS, SCSS, Less, JavaScript, TypeScript, TSX, PHP, JSON, Markdown, XML/SVG, Python, Ruby, Rust, Go, Java, C, C++, C#, Shell, SQL, Lua, YAML, TOML, INI/.htaccess und Diff. Eingebettete Sprachen werden mit gefärbt (CSS/JS in HTML, HTML in PHP, Code-Blöcke in Markdown).
- Autovervollständigung: HTML-Tags und -Attribute, CSS-Eigenschaften und -Werte, Schlüsselwörter, Snippets (z. B. `html5`, `media`, `log`) und Wörter aus dem Dokument. Eigene Einträge pro Sprache.
- Emmet mit Tab, z. B. `ul.nav>li.item$*3>a{Link $}`
- Mehrere Cursor, Suchen und Ersetzen (auch per Regex, auf Wunsch nur in der Auswahl – bei mehrzeiliger Auswahl automatisch), Projektsuche mit Ersetzen, Dateimustern (`*.php, !vendor/`) und Beachtung der `.gitignore` – auch auf Servern über SSH (nutzt `rg`, `git grep` oder `grep` auf dem Server; Rechtsklick auf einen Ordner: „In diesem Ordner suchen…“). Ort und Ordner der Suche folgen der zuletzt genutzten Ansicht (Dateien oder Server) und der Auswahl dort; laufende Suchen lassen sich abbrechen
- Schnell öffnen (Strg+P): unscharfe Suche über die Projektdateien und die Dateien in bereits geladenen Serverordnern (Servername grau dahinter); `index.php:42` bzw. `:42:5` öffnet direkt in Zeile (und Spalte), nur `:42` springt im aktuellen Tab
- Code-Faltung mit +/−-Knöpfen in der Zeilennummernleiste: nach Klammern und HTML-Tags (auch über PHP-Abschnitte hinweg), sonst nach Einrückung
- Automatischer Zeilenumbruch (Ansicht, Alt+Z); Fortsetzungszeilen behalten die Einrückung und haben keine eigene Zeilennummer
- Drucken mit Syntaxfarben, Zeilennummern und Seitenzahlen (Strg+Alt+P)
- Farbvorschau unter CSS-Farbwerten; ein Klick darauf öffnet einen Farbwähler, der den Wert in seiner Schreibweise (Hex, rgb(), hsl()) zurückschreibt
- Bildvorschau: PNG, JPG, GIF, WebP, BMP und ICO öffnen als Tab mit Abmessungen und Dateigröße (lokal und vom Server; Klick wechselt zwischen eingepasst und 100 %); SVG bleibt bearbeitbarer Text mit Vorschau im rechten Panel
- Pfadleiste mit Position im Dokument (z. B. `body › main#content › ul`, sonst die Funktion/Klasse am Cursor); Klick auf Ordner oder Datei zeigt die Nachbarn zum Springen (auch auf dem Server), Klick auf das Symbol die Gliederung
- HTML-Tags und Klammern werden automatisch geschlossen
- Die Sitzung wird beim nächsten Start wiederhergestellt (geöffnete Dateien und Cursor)
- Wiederherstellung nach Absturz: Ungespeicherte Inhalte (auch unbenannte Tabs) werden alle paar Sekunden gesichert und nach einem Absturz beim nächsten Start zurückgeholt
- Absturzbericht: Stürzt Nebula ab, landet ein Bericht (Meldung, Codestelle, Backtrace – keine Dateiinhalte oder Zugangsdaten) im Ordner `crashes` der Konfiguration; beim nächsten Start bietet ein Dialog an, ihn zu öffnen, zu kopieren oder im Explorer zu zeigen. Die letzten 10 Berichte bleiben erhalten
- Automatisch speichern (Einstellungen → Editor): nach einer Tipppause oder beim Wechsel von Tab bzw. Fenster, Serverdateien nur auf Wunsch
- Lokaler Verlauf: Beim Öffnen und bei jedem Speichern (auch auf dem Server) wird eine Fassung aufbewahrt; über das Tab-Menü „Lokaler Verlauf…“ mit dem aktuellen Stand vergleichen und wiederherstellen. Beim Neuladen verworfene Änderungen bleiben dort ebenfalls erhalten
- Externe Änderungen: Wird eine geöffnete Datei von einem anderen Programm geändert (z. B. Git), lädt Nebula sie neu (rückgängig machbar); bei eigenen ungespeicherten Änderungen wird nachgefragt, mit Unterschiede-Ansicht. Gelöschte Dateien werden gemeldet, die Seitenleiste aktualisiert sich selbst.
- Geteilte Ansicht (Knopf rechts in der Pfadleiste, daneben einer für die Minimap; Strg+Alt+T, Tab-Kontextmenü): zwei Dateien oder dieselbe Datei an zwei Stellen nebeneinander
- Zen-Modus (Strg+Alt+Enter oder F11, Esc beendet): nur der Editor, Vollbild, mittige Textspalte
- Minimap am rechten Rand des Editors (Token-Farben, sichtbarer Bereich hervorgehoben, Klicken/Ziehen scrollt; abschaltbar unter Einstellungen → Editor)
- Editorschrift frei wählbar (alle installierten Schriften, mit Suche) und Zeilenhöhe einstellbar (Einstellungen → Editor)
- Gehe zu Symbol (Strg+R) als Schnellsuche und Navigationsverlauf: Alt+←/→ springt zu früheren Cursorpositionen, auch über Dateien hinweg
- Passendes HTML-Tag wird hervorgehoben, beim Umbenennen ändert sich das schließende Tag mit; gleiche Wörter wie das unter dem Cursor werden dezent markiert
- Gliederung im rechten Panel: Funktionen, Klassen, CSS-Regeln, Überschriften und Elemente mit ID der aktuellen Datei, filterbar; ein Klick springt hin, das Symbol am Cursor ist markiert
- Blockauswahl mit Alt+Ziehen; Alt+Umschalt+I setzt Cursor an alle Zeilenenden der Auswahl
- Textwerkzeuge (Kontextmenü „Text“ und Befehlspalette): Zeilen sortieren (natürlich, ohne Groß-/Kleinschreibung), doppelte entfernen, umkehren, verbinden (Strg+J), Groß-/Kleinschreibung, Auswahl mit Tag umschließen (Alt+W, Name für beide Tags gleichzeitig tippen)
- Git-Markierungen in der Zeilennummernleiste: neue, geänderte und gelöschte Zeilen gegenüber dem letzten Commit
- Vergleichen als eigener Tab (Unified-Diff, schreibgeschützt, mit Strg+S als .diff speicherbar): Arbeitsstand gegen den letzten Commit, Tab gegen eine Datei, zwei Dateien aus der Seitenleiste (lokal und auf dem Server gemischt) – offene Tabs werden mit ihrem aktuellen Stand verglichen
- Git-Tab in der Seitenleiste: Dateien vormerken und zurücknehmen, Commit mit Nachricht (Strg+Enter; ohne Vormerkung werden alle Änderungen committet), Pull/Push mit Anzeige der Commits, Abrufen, Branch wechseln und neu anlegen; Klick auf eine Datei zeigt ihre Änderungen
- Übertragungsliste in der Statusleiste: Uploads, Downloads, Kopien und Speichervorgänge auf Servern mit Fortschritt (Dateien), Abbrechen, Wiederholen bei Fehlern und „Erledigte entfernen“
- Übersicht am rechten Rand: Suchtreffer, Probleme, Git-Änderungen, Lesezeichen und Cursor der ganzen Datei
- `.editorconfig` wird ausgewertet (Einrückung, Zeilenenden, Kodierung, Leerzeichen am Zeilenende, Zeilenumbruch am Dateiende); beides auch als Einstellung beim Speichern. Einrückung lässt sich zwischen Tabs und Leerzeichen umwandeln.

**Live-Vorschau**
- HTML-, PHP- und Markdown-Seiten sowie Bilder im rechten Panel (Knopf rechts in der Titelleiste, Strg+Umschalt+V; WebView2), aktualisiert beim Tippen, auch mit ungespeicherten Änderungen
- Geänderte CSS-Dateien werden ohne Neuladen der Seite übernommen, die Scrollposition bleibt erhalten
- Funktioniert auch für Dateien auf dem Server: Bilder, CSS und Skripte werden über die Serververbindung geladen
- PHP-Dateien mit hinterlegter Web-URL zeigen die Live-Seite und laden nach dem Speichern neu
- Zurück/Vor nach gefolgten Links, Neu laden
- Entwicklertools (F12-Konsole der Seite) und „Im Browser öffnen“ (Standardbrowser; unbenannte Dateien und Serverdateien ohne Web-URL als temporäre Kopie)

**KI-Agenten**
- Coding-Agenten im rechten Panel (Reiter „KI“, Strg+Umschalt+A): Claude Code, Codex (ChatGPT) und Gemini CLI über das Agent Client Protocol (ACP, wie in Zed); eigene ACP-Agenten lassen sich hinzufügen
- Der Agent arbeitet im geöffneten Projekt: Antworten als Markdown, Gedanken, Plan, Werkzeugaufrufe mit Diffs; vor Änderungen und Befehlen fragt er nach (Erlauben / Immer erlauben / Ablehnen). Änderungen landen rückgängig machbar im offenen Tab und werden gespeichert; mit „Beim Speichern hochladen“ gehen sie auch auf den Server
- Eingabe mit Werkzeugleiste: Agent, Modell, Aufwand und weitere Einstellungen des Agenten (Modell und Aufwand werden pro Agent gemerkt), Modus darunter; Stopp per ■ oder Esc
- Dateien anhängen: Büroklammer (aktuelle Datei, Dateiauswahl) oder Rechtsklick in der Seitenleiste „An KI-Agent anhängen“, auch für Serverdateien
- Verbleibendes 5-Stunden- und Wochenlimit des Claude-Abos unter der Eingabe (ohne Tokenverbrauch)
- Einstellungen → KI-Agenten: Status prüfen, Anmelden mit den Methoden des Anbieters (Browser-Login mit Claude-, ChatGPT- oder Google-Konto, oder API-Schlüssel in der Windows-Anmeldeinformationsverwaltung), Abmelden
- Node.js: ein installiertes Node wird genutzt, sonst lädt Nebula auf Wunsch die aktuelle LTS-Version (SHA-256-geprüft) nach `%APPDATA%\nebula\runtime`; die Agenten liegen in `%APPDATA%\nebula\agents`

**Erweiterungen (Nova-kompatibel)**
- Installation direkt aus Novas Extension Library (Suche, Kategorien, Beschreibung), aus einem Ordner oder einer ZIP-Datei
- JavaScript-Laufzeit (QuickJS) mit der Nova-Extension-API: Befehle, Einstellungen, Editor- und Dokument-API, Meldungen, Paletten, Benachrichtigungen, Prozesse (auch JSON-RPC), fetch, Dateisystem, Pfade, Zwischenablage, Anmeldedaten, Timer, Seitenleisten (TreeView)
- Language Server (LanguageClient): Probleme, Vervollständigung, Hover-Infos, Gehe zu Definition (Strg+Klick oder F12)
- Formatierer über `onWillSave`, Prüf-Assistenten (Issues) mit Wellenlinien und Problemliste in der Statusleiste
- Einstellungsseiten werden aus den Manifesten der Erweiterungen erzeugt; Projekteinstellungen liegen wie bei Nova in `.nova/Configuration.json`
- Clips und Themes aus Erweiterungen (Themes erscheinen automatisch in der Theme-Auswahl); Erweiterungskonsole für Ausgaben und Fehler
- macOS-typische Aufrufe werden übersetzt (`/usr/bin/env`, Shell-Skripte über Git Bash, npm-Shims). Erweiterungen mit macOS-Programmen oder Tree-sitter-Grammatiken als `.dylib` laufen unter Windows nicht.

**Oberfläche**
- Eigene Titelleiste mit Tabs; Tabs scrollen, lassen sich per Drag sortieren und haben ein Kontextmenü (Speichern, Duplizieren, Drucken, Schließen, andere/rechts/alle schließen, Pfad kopieren, im Explorer zeigen)
- Windows-11-Andocklayouts am Maximieren-Knopf, Mica-Hintergrund in der Seitenleiste, native Schatten und abgerundete Ecken
- Seitenleiste mit Dateien, Servern und Projektsuche; Git-Status (Branch, geänderte und neue Dateien)
- Zuletzt geöffnete Projekte: Klick auf den Projektnamen in der Seitenleiste, Menü, Startseite oder Befehlspalette. Pro Projekt merkt sich Nebula die offenen Dateien. Die Listen der zuletzt geöffneten Projekte und Dateien lassen sich auf der Startseite („Leeren“) und im Datei-Menü leeren.
- Befehlspalette mit allen Befehlen
- Themes „Nebula Dunkel“ und „Nebula Hell“ (Paletten nach Novas Themes). Eigene Themes lassen sich bearbeiten, als JSON importieren und exportieren, und Nova-Themes (.css) können importiert werden. Für helles und dunkles Design ist je ein Theme wählbar; umgeschaltet wird in den Einstellungen, im Menü „Ansicht → Erscheinungsbild“ oder per Knopf in der Titelleiste (Sonne, Mond oder automatisch wie Windows).
- Einstellungen für Editor, Design und Sprachen (Dateiendungen, Einrückung, eigene Vervollständigungen)
- Eigene Tastenkürzel (Einstellungen → Tastenkürzel): neue Kombination aufnehmen, weitere hinzufügen, auf Standard zurücksetzen; doppelt belegte Kürzel werden rot markiert

## Tastenkürzel

Alle Kürzel lassen sich unter Einstellungen → Tastenkürzel ändern. Standardbelegung:

| Aktion | Kürzel |
|---|---|
| Befehlspalette | Strg+Umschalt+P |
| Neu / Öffnen / Ordner öffnen / Schnell öffnen | Strg+N / Strg+O / Strg+Umschalt+O / Strg+P |
| Speichern / Speichern unter / Alle speichern | Strg+S / Strg+Umschalt+S / Strg+Alt+S |
| Tab schließen / Tab wechseln | Strg+W / Strg+Tab |
| Suchen / Ersetzen / Weitersuchen | Strg+F / Strg+H / F3, Umschalt+F3 |
| Im Projekt suchen | Strg+Umschalt+F |
| Gehe zu Zeile / Gehe zu Symbol | Strg+G / Strg+R |
| Navigation zurück / vor | Alt+← / Alt+→ (auch Maustasten 4/5) |
| Lesezeichen setzen / nächstes / vorheriges | Strg+Alt+K / Strg+Alt+L / Strg+Alt+J (oder Strg+F2 / F2 / Umschalt+F2) |
| Im Browser öffnen | F5 |
| Rechtes Panel mit Live-Vorschau ein/aus | Strg+Umschalt+V |
| KI-Agent ein/aus | Strg+Umschalt+A |
| Automatischer Zeilenumbruch | Alt+Z |
| Drucken | Strg+Alt+P |
| Zen-Modus | Strg+Alt+Enter / F11 |
| Geteilte Ansicht | Strg+Alt+T |
| Blockauswahl / Cursor an Zeilenenden | Alt+Ziehen / Alt+Umschalt+I |
| Terminal ein/aus | Strg+Ö |
| Gehe zu Definition (Erweiterungen) | F12 / Strg+Klick |
| Vervollständigung / Emmet | Strg+Leertaste / Tab |
| Nächstes / alle Vorkommen auswählen | Strg+D / Strg+Umschalt+L |
| Cursor darüber/darunter | Strg+Alt+↑/↓ |
| Zeile verschieben / duplizieren / löschen | Alt+↑/↓ / Strg+Umschalt+D / Strg+Umschalt+K |
| Kommentar umschalten | Strg+/ |
| Seitenleiste / Einstellungen | Strg+B / Strg+, |
| Dateien (Seitenleiste): Kopieren, Ausschneiden, Einfügen, Duplizieren | Strg+C / Strg+X / Strg+V / Strg+D |
| Dateien: Umbenennen / Löschen | F2 / Entf |

## Architektur

```
src/
  main.rs              Fenster, Schriften
  app.rs               Anwendungszustand, Layout, Menüs, Dialoge
  app/files.rs         Kopieren/Verschieben/Upload/Download, Drops aus dem Explorer
  app/publish.rs       Veröffentlichen, Browser-Vorschau
  app/preview.rs       Live-Vorschau (Seitenwahl, Auslieferung, Neuladen)
  app/agent.rs         KI-Agent im rechten Panel (Unterhaltung, Eingabe, Anhänge, Limits)
  app/agents_page.rs   Einstellungsseite KI-Agenten (Anmeldung, Node.js)
  acp/                 Agent Client Protocol: Verbindung, Sitzung, Node-Verwaltung, Limits
  app/extensions.rs    Erweiterungen in der App: Dokument-Abgleich, Anfragen, Probleme
  app/extensions_page.rs  Einstellungsseite: installierte Erweiterungen, Bibliothek, Konsole
  preview.rs           WebView2-Fenster und eigenes URL-Schema
  ext/                 Erweiterungs-Host: Manifeste, QuickJS-Laufzeit (runtime.js), Prozesse, Bibliothek
  app/palette.rs       Befehlspalette
  app/settings_page.rs Einstellungsfenster
  app/session.rs       Sitzung speichern und wiederherstellen
  app/tabs.rs          Tab-Kontextmenü, Duplizieren, Drucken
  app/terminal.rs      Terminal-Panel (lokal und SSH): Tabs, Darstellung, Tastatur
  print.rs             Druckdialog und Seitenaufbau (GDI)
  win32.rs             Mica, Andocklayouts, Datei-Zwischenablage, Cursorposition, Browser öffnen, Dateien hinausziehen
  ui/                  Titelleiste, Seitenleiste, Projektsuche, Git, Icons
  editor/              Dokument, Befehle, Vervollständigung, Emmet, Faltung, Suche, Widget
  highlight.rs         tree-sitter-Highlighting (Grammatiken werden bei Bedarf geladen)
  queries/             Eigene Abfragen für HTML und CSS mit Nova-Kategorien
  remote/              FTP/FTPS (suppaftp) und SFTP (russh), Worker pro Server, SSH-Shell (terminal.rs, vt100)
  theme.rs             Themes, Theme-Dateien
  theme_import.rs      Import von Nova-Themes
  settings.rs          Einstellungen, Server, Sprachen, Veröffentlichen
```
