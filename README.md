# Nebula

Schneller, nativer Code-Editor für Windows, inspiriert von Panic Nova. In Rust geschrieben. Schwerpunkt ist das direkte Bearbeiten von Websites auf Servern per FTP, FTPS und SFTP.

## Bauen und starten

Voraussetzungen: Rust (stable, MSVC-Toolchain) und die Visual Studio Build Tools (für die tree-sitter-Grammatiken in C).

```powershell
cargo run --release                     # starten
cargo run --release -- C:\pfad\projekt  # Ordner direkt öffnen
cargo test                              # Unit- und Integrationstests (inkl. FTP/SFTP)
$env:NEBULA_PERF_FILE='C:gross.js'; cargo test --profile perf bench_file -- --ignored --nocapture  # Zeiten für eine große Datei
```

Das Ergebnis ist eine einzelne Datei ohne weitere Abhängigkeiten: `target\release\nebula.exe`.

## Funktionen

**Server und Dateien**
- Server-Verwaltung für SFTP, FTP und FTPS (explizites oder implizites TLS über Windows SChannel)
- Sicherungskopie auf dem Server (pro Server einschaltbar): vor dem Überschreiben beim Speichern oder Hochladen wird die vorhandene Datei als `name.bak` gesichert
- Offline-Ansicht für Server: geöffnete und gespeicherte Serverdateien sowie aufgeklappte Ordner werden zwischengespeichert (Ordner `offline` im Einstellungsordner, bis 500 MB). Ist ein Server nicht erreichbar, zeigt die Seitenleiste den letzten Stand und Dateien öffnen sich schreibgeschützt aus der Kopie (Hinweis in der Statusleiste); nach dem Wiederverbinden werden sie neu geladen. Abschaltbar und leerbar unter Einstellungen › Editor
- Server in einklappbaren Gruppen ordnen (Feld „Gruppe“ im Serverdialog oder Rechtsklick → „Gruppe“; Gruppen per Rechtsklick umbenennen oder auflösen, oder per Drag & Drop auf einen Gruppenkopf bzw. einen Server der Gruppe; auf „Server“ gezogen ohne Gruppe), Server duplizieren; Umschalter in der Kopfzeile zeigt alle Server ohne Gruppen alphabetisch
- Import aus FileZilla (`sitemanager.xml`, Ordner werden zu Gruppen, Passwörter auf Wunsch) und aus Nebula-Exporten; Export der Serverliste als JSON ohne Passwörter. Bereits vorhandene Server werden erkannt und nicht doppelt angelegt (≡-Menü im Reiter „Server“, Menü „Server“ oder Befehlspalette)
- Terminal im Panel unter dem Editor (Strg+Ö oder Knopf in der Titelleiste): lokale Shells (PowerShell, Windows PowerShell, Git Bash, Eingabeaufforderung) im Projektordner, z. B. für Claude Code oder npm; Standard-Shell in den Einstellungen wählbar. SSH-Terminals über das +-Menü, das Kontextmenü eines Servers oder die Befehlspalette: mehrere Sitzungen als Tabs, 256 Farben, Scrollback per Mausrad, Suche im Verlauf (Strg+F: alle Treffer markiert, Enter springt zum älteren, Umschalt+Enter zum neueren), Markieren und Kopieren, Einfügen, Neuverbinden mit Enter. Nutzt Zugangsdaten und Host-Key-Prüfung der Server (bei FTP-Servern Port 22) und wechselt in den Startpfad.
- Datenbank light (Rechtsklick auf einen Server › „Datenbank öffnen“ oder Befehlspalette): MySQL/MariaDB über einen SSH-Tunnel mit der Anmeldung des Servers – kein `mysql` auf dem Server nötig. Tabellenliste mit Filter, ein Klick zeigt die ersten 200 Zeilen, eigene Abfragen mit Strg+Enter, Ergebnis als Tabelle (bis 1000 Zeilen, NULL grau, Zeile kopieren, als CSV kopieren). Abfragen, die etwas ändern, müssen erst bestätigt werden. Zugangsdaten pro Server, Passwort in der Windows-Anmeldeinformationsverwaltung
- Gespeicherte Serverbefehle (z. B. Cache leeren, Deploy): im SSH-Terminal über „Befehle ▾“ anlegen und ausführen, außerdem per Rechtsklick auf den Server („Befehl ausführen“) und in der Befehlspalette; öffnet bei Bedarf ein Terminal und führt den Befehl im Startordner aus
- Berechtigungen (chmod) für Dateien und Ordner auf dem Server, auf Wunsch rekursiv (nur Dateien, nur Ordner oder alles); die aktuellen Rechte stehen im Tooltip
- Dateien auf dem Server öffnen und mit Strg+S direkt hochladen
- Konflikterkennung: Wurde eine Datei auf dem Server inzwischen geändert, fragt Nebula nach (Überschreiben, Server-Version laden oder Abbrechen) und zeigt auf Wunsch die Unterschiede an
- Kopieren, Ausschneiden, Einfügen, Duplizieren, Umbenennen und Löschen, lokal und auf Servern, auch mit Mehrfachauswahl (Strg+Klick); vorhandene Namen werden beim Anlegen und Umbenennen nicht überschrieben, lokal Gelöschtes landet im Papierkorb (Netzlaufwerke haben keinen – dann fragt der Dialog nach endgültigem Löschen)
- Drag & Drop im Baum: verschiebt, mit Strg wird kopiert. Ordner öffnen sich beim Darüberhalten. Lokale Dateien lassen sich aus dem Fenster heraus in den Explorer (oder andere Programme) ziehen.
- Dateien aus dem Explorer in einen Ordner ziehen oder dort einfügen: lokal kopieren oder auf den Server hochladen. In Nebula kopierte Dateien lassen sich im Explorer einfügen.
- Zwischen lokal und Server sowie zwischen zwei Servern wird automatisch hoch- bzw. heruntergeladen, Ordner rekursiv. Ziehen zwischen verschiedenen Orten kopiert, innerhalb eines Orts wird verschoben (Umschalt: verschieben, Strg: kopieren; der Hinweis am Mauszeiger zeigt es an); im Kontextmenü außerdem „Auf Server kopieren ›“ und „Auf Server verschieben ›“ (je Server in den dort zuletzt angeklickten Ordner, in den Startordner oder über „In Ordner…“ in einen frei gewählten Serverordner) sowie bei Serverdateien „Herunterladen in Projektordner ›“ (ausgewählter Projektordner, Projektordner oder „In Ordner…“). Bei Namenskonflikten fragt ein Dialog: Ersetzen, Beide behalten oder Überspringen – auch wenn der Serverordner noch nicht aufgeklappt war.
- Veröffentlichen: Ein lokaler Projektordner wird mit einem Server-Ordner verknüpft. Hochgeladen werden nur geänderte Dateien, auf Wunsch automatisch beim Speichern.
- Synchronisieren (Rechtsklick auf den Veröffentlichen-Knopf oder Befehlspalette): gleicht Projekt und Serverordner in beide Richtungen ab. Eine Vorschau zeigt Hochladen, Herunterladen, Löschen und Konflikte (auf beiden Seiten geändert) zum Abhaken; Löschungen sind nicht vorausgewählt, lokale landen im Papierkorb. Nebula merkt sich den Stand nach jeder Synchronisierung; Ausschlussliste (z. B. `.git`, `node_modules`, `.env`) im Dialog „Veröffentlichen einrichten“. Lässt sich eine Seite nicht vollständig einlesen (Ordner nicht lesbar, mehr als 50 000 Dateien), wird nicht synchronisiert. Beim ersten Abgleich werden gleich große Dateien inhaltlich verglichen
- Passwörter liegen in der Windows-Anmeldeinformationsverwaltung. SFTP-Host-Keys werden beim ersten Verbinden bestätigt und danach geprüft.

**Editor**
- 26 Sprachen mit tree-sitter-Highlighting: HTML, CSS, SCSS, Less, JavaScript, TypeScript, TSX, PHP, JSON, Markdown, XML/SVG, Python, Ruby, Rust, Go, Java, C, C++, C#, Shell, SQL, Lua, YAML, TOML, INI/.htaccess und Diff. Eingebettete Sprachen werden mit gefärbt (CSS/JS in HTML, HTML in PHP, Code-Blöcke in Markdown).
- Autovervollständigung: HTML-Tags und -Attribute, CSS-Eigenschaften und -Werte, Schlüsselwörter, Snippets (z. B. `html5`, `media`, `log`) und Wörter aus dem Dokument. Eigene Einträge pro Sprache.
- Snippets mit Platzhaltern (`$1`, `${2:Name}`, `$0`, Variablen wie `$SELECTED_TEXT`, `$FILENAME`, `$DATE`): nach dem Einfügen springt Tab von Platzhalter zu Platzhalter, gleiche Nummern werden gemeinsam bearbeitet, Esc beendet. Im rechten Panel unter „{ }“ stehen die Snippets der aktuellen Sprache (eigene und eingebaute) – Klick fügt ein, „+“ legt ein neues an (mit der Auswahl als Inhalt), eingebaute lassen sich per Rechtsklick als eigene anpassen; eigene gelten für eine Sprache oder alle und lassen sich in einklappbare Gruppen ordnen (Feld „Gruppe“ oder per Drag & Drop auf einen Gruppenkopf). Über „⋯“ lassen sich alle eigenen Snippets als JSON exportieren und wieder importieren – auch VS-Code-Snippets (`html.json`, `*.code-snippets`, der Dialog startet in VS Codes Snippet-Ordner); Doppelte werden erkannt
- Emmet mit Tab, z. B. `ul.nav>li.item$*3>a{Link $}`
- Mehrere Cursor, Suchen und Ersetzen (auch per Regex, auf Wunsch nur in der Auswahl – bei mehrzeiliger Auswahl automatisch), Projektsuche mit Ersetzen, Dateimustern (`*.php, !vendor/`) und Beachtung der `.gitignore` – auch auf Servern über SSH (nutzt `rg`, `git grep` oder `grep` auf dem Server; Rechtsklick auf einen Ordner: „In diesem Ordner suchen…“). Ort und Ordner der Suche folgen der zuletzt genutzten Ansicht (Dateien oder Server) und der Auswahl dort; laufende Suchen lassen sich abbrechen
- Vorschau beim Ersetzen: die Suchleiste zeigt, was aus dem aktuellen Treffer wird, die Projektsuche zeigt jeden Treffer als „alt → neu“; bei Regex Gruppen mit `$1` oder `\1`, `${name}`, dazu `\n` und `\t`
- Schnell öffnen (Strg+P): unscharfe Suche über die Projektdateien und die Dateien in bereits geladenen Serverordnern (Servername grau dahinter); `index.php:42` bzw. `:42:5` öffnet direkt in Zeile (und Spalte), nur `:42` springt im aktuellen Tab
- Code-Faltung mit +/−-Knöpfen in der Zeilennummernleiste: nach Klammern und HTML-Tags (auch über PHP-Abschnitte hinweg), sonst nach Einrückung
- Automatischer Zeilenumbruch (Ansicht, Alt+Z); Fortsetzungszeilen behalten die Einrückung und haben keine eigene Zeilennummer
- Drucken mit Syntaxfarben, Zeilennummern und Seitenzahlen (Strg+Alt+P)
- Farbvorschau unter CSS-Farbwerten; ein Klick darauf öffnet einen Farbwähler, der den Wert in seiner Schreibweise (Hex, rgb(), hsl()) zurückschreibt
- Bildvorschau: PNG, JPG, GIF, WebP, BMP und ICO öffnen als Tab mit Abmessungen und Dateigröße (lokal und vom Server; Klick wechselt zwischen eingepasst und 100 %); SVG bleibt bearbeitbarer Text mit Vorschau im rechten Panel
- Hex-Editor für Binärdateien: Offset, Bytes und Zeichen; Bytes per Hex-Ziffern oder als Text überschreiben oder einfügen (Einfg wechselt den Modus, Entf/Rück löschen; bei Dateien bis 64 MB, Tab wechselt die Spalte), geänderte Bytes farbig, Strg+Z/Strg+Y, Gehe zu Offset (Strg+G, `0x1F00`, `1F00h` oder dezimal), Suche nach Hex-Bytes oder Text (Strg+F, Enter/Umschalt+Enter). Strg+S schreibt lokal nur die geänderten Bytes in die Datei (auch bei sehr großen Dateien, die stückweise gelesen werden; nach Einfügen/Löschen die ganze Datei), Serverdateien werden hochgeladen; „Als Text öffnen“ falls die Datei doch Text ist. Textdateien ab 16 MB laden im Hintergrund (Tab zeigt „Lade …“), über 64 MB öffnen sie schreibgeschützt (bis 512 MB, darüber als Hex)
- Pfadleiste mit Position im Dokument (z. B. `body › main#content › ul`, sonst die Funktion/Klasse am Cursor); Klick auf Ordner oder Datei zeigt die Nachbarn zum Springen (auch auf dem Server), Klick auf das Symbol die Gliederung
- HTML-Tags und Klammern werden automatisch geschlossen
- Die Sitzung wird beim nächsten Start wiederhergestellt (geöffnete Dateien und Cursor)
- Wiederherstellung nach Absturz: Ungespeicherte Inhalte (auch unbenannte Tabs) werden alle paar Sekunden gesichert und nach einem Absturz beim nächsten Start zurückgeholt
- Absturzbericht: Stürzt Nebula ab, landet ein Bericht (Meldung, Codestelle, Backtrace – keine Dateiinhalte oder Zugangsdaten) im Ordner `crashes` der Konfiguration; beim nächsten Start bietet ein Dialog an, ihn zu öffnen, zu kopieren oder im Explorer zu zeigen. Auch harte Abstürze in nativem Code (z. B. Zugriffsverletzung in einem Grafiktreiber) werden erfasst, mit Ausnahmecode, betroffenem Modul und einem Speicherabbild (`.dmp`) für einen Debugger – das Abbild enthält Teile des Arbeitsspeichers und sollte nur an Vertrauenswürdige weitergegeben werden. Die letzten 10 Berichte bleiben erhalten (Testauslöser: `NEBULA_CRASH_TEST=1` bzw. `=native`)
- Automatisch speichern (Einstellungen → Editor): nach einer Tipppause oder beim Wechsel von Tab bzw. Fenster, Serverdateien nur auf Wunsch
- Lokaler Verlauf: Beim Öffnen und bei jedem Speichern (auch auf dem Server) wird eine Fassung aufbewahrt; über das Tab-Menü „Lokaler Verlauf…“ mit dem aktuellen Stand vergleichen und wiederherstellen. Beim Neuladen verworfene Änderungen bleiben dort ebenfalls erhalten
- Externe Änderungen: Wird eine geöffnete Datei von einem anderen Programm geändert (z. B. Git), lädt Nebula sie neu (rückgängig machbar); bei eigenen ungespeicherten Änderungen wird nachgefragt, mit Unterschiede-Ansicht. Gelöschte Dateien werden gemeldet, die Seitenleiste aktualisiert sich selbst.
- Geteilte Ansicht (Knopf rechts in der Pfadleiste, daneben einer für die Minimap; Strg+Alt+T, Tab-Kontextmenü): zwei Dateien oder dieselbe Datei an zwei Stellen nebeneinander
- Zen-Modus (Strg+Alt+Enter oder F11, Esc beendet): nur der Editor, Vollbild, mittige Textspalte
- Minimap am rechten Rand des Editors (Token-Farben, sichtbarer Bereich hervorgehoben, Klicken/Ziehen scrollt; abschaltbar unter Einstellungen → Editor)
- Makros (Menü Bearbeiten, Befehlspalette): Tippen, Einfügen, Tasten und Bearbeiten-Befehle wie „Zeile duplizieren“ oder „Nächster Treffer“ aufnehmen (Strg+Umschalt+R, Anzeige in der Statusleiste) und abspielen (Strg+Umschalt+M) oder mehrmals abspielen; Esc hält eine lange Wiedergabe an
- Rechtschreibprüfung mit der Windows-Rechtschreibprüfung: Kommentare in Code, Text in Markdown (ohne Code und Links), HTML und Textdateien; code-artige Wörter (camelCase, Pfade, Adressen) werden übersprungen. Rechtsklick auf ein markiertes Wort zeigt Vorschläge, „Zum Wörterbuch hinzufügen“ und „Ignorieren“. Sprachen automatisch (Windows-Sprache und Englisch, soweit installiert) oder wählbar unter Einstellungen › Editor
- Editorschrift frei wählbar (alle installierten Schriften, mit Suche) und Zeilenhöhe einstellbar (Einstellungen → Editor); Ligaturen von Schriften wie Cascadia Code, Fira Code oder JetBrains Mono (`=>`, `!=`, `->` …), abschaltbar
- Gehe zu Symbol (Strg+R) als Schnellsuche und Navigationsverlauf: Alt+←/→ springt zu früheren Cursorpositionen, auch über Dateien hinweg
- Passendes HTML-Tag wird hervorgehoben, beim Umbenennen ändert sich das schließende Tag mit; gleiche Wörter wie das unter dem Cursor werden dezent markiert
- Gliederung im rechten Panel: Funktionen, Klassen, CSS-Regeln, Überschriften und Elemente mit ID der aktuellen Datei, filterbar; ein Klick springt hin, das Symbol am Cursor ist markiert
- Blockauswahl mit Alt+Ziehen; Alt+Umschalt+I setzt Cursor an alle Zeilenenden der Auswahl
- Textwerkzeuge (Kontextmenü „Text“ und Befehlspalette): Zeilen sortieren (natürlich, ohne Groß-/Kleinschreibung), doppelte entfernen, umkehren, verbinden (Strg+J), Groß-/Kleinschreibung, Auswahl mit Tag umschließen (Alt+W, Name für beide Tags gleichzeitig tippen)
- Git-Markierungen in der Zeilennummernleiste: neue, geänderte und gelöschte Zeilen gegenüber dem letzten Commit
- Blame für die aktuelle Zeile: Autor, Alter und Commit-Nachricht dezent am Zeilenende, Details (Datum, Hash) beim Darüberfahren; geänderte Zeilen zeigen „Noch nicht committet“ (Menü Ansicht oder Einstellungen › Editor)
- Vergleichen als eigener Tab (Unified-Diff, schreibgeschützt, mit Strg+S als .diff speicherbar): Arbeitsstand gegen den letzten Commit, Tab gegen eine Datei, zwei Dateien aus der Seitenleiste (lokal und auf dem Server gemischt) – offene Tabs werden mit ihrem aktuellen Stand verglichen
- Git-Tab in der Seitenleiste: Dateien vormerken und zurücknehmen, Commit mit Nachricht (Strg+Enter; ohne Vormerkung werden alle Änderungen committet), Pull/Push mit Anzeige der Commits, Abrufen, Branch wechseln und neu anlegen; Klick auf eine Datei zeigt ihre Änderungen
- Commit-Nachricht per KI: ✦ im Nachrichtenfeld des Git-Tabs lässt den Agenten aus dem Panel eine Nachricht aus dem Diff schreiben (vorgemerkte Änderungen, sonst alle), in Sprache und Stil der letzten Commits; in einer eigenen Sitzung, die Unterhaltung im Panel bleibt unberührt
- Nebula als MCP-Server (Einstellungen › KI-Agenten): Agenten sehen offene Dateien, den aktiven Editor mit Cursor und Auswahl, gemeldete Probleme und ungespeicherte Texte und können Dateien im Editor öffnen. Nur lokal erreichbar, mit Schlüssel in der Adresse; die Agenten im Panel bekommen ihn automatisch, für Claude Code im Terminal gibt es den fertigen `claude mcp add`-Befehl zum Kopieren
- Inline-Vorschläge beim Tippen (Einstellungen › KI-Agenten, standardmäßig aus): nach einer kurzen Pause am Zeilenende erscheint grau ein Vorschlag eines eingebauten Agenten (API-Schlüssel oder Ollama, automatisch ein kleines, schnelles Modell); Tab übernimmt, Esc verwirft
- Übertragungsliste in der Statusleiste: Uploads, Downloads, Kopien und Speichervorgänge auf Servern mit Fortschritt (Dateien), Abbrechen, Wiederholen bei Fehlern und „Erledigte entfernen“
- Übersicht am rechten Rand: Suchtreffer, Probleme, Git-Änderungen, Lesezeichen und Cursor der ganzen Datei
- Lesezeichen: Doppelklick auf eine Zeilennummer (oder Strg+F2) setzt bzw. entfernt eins; der Reiter „Lesezeichen“ im rechten Panel listet sie für alle offenen Dateien mit Zeilentext, Klick springt hin, × entfernt; Lesezeichen bleiben über einen Neustart erhalten
- `.editorconfig` wird ausgewertet (Einrückung, Zeilenenden, Kodierung, Leerzeichen am Zeilenende, Zeilenumbruch am Dateiende); beides auch als Einstellung beim Speichern. Einrückung lässt sich zwischen Tabs und Leerzeichen umwandeln.

**Live-Vorschau**
- HTML-, PHP- und Markdown-Seiten sowie Bilder im rechten Panel (Knopf rechts in der Titelleiste, Strg+Umschalt+V; WebView2), aktualisiert beim Tippen, auch mit ungespeicherten Änderungen
- Geänderte CSS-Dateien werden ohne Neuladen der Seite übernommen, die Scrollposition bleibt erhalten
- Gerätebreiten: Panelbreite, Handy (390 px), Tablet (768 px) oder Desktop (1440 px) über den Geräte-Knopf der Vorschau, Handy und Tablet auch im Querformat (844 bzw. 1024 px), dazu eigene Breiten (im Menü hinzufügen und entfernen); ist das Panel schmaler, wird die Seite verkleinert dargestellt, Media Queries greifen wie auf dem Gerät
- Funktioniert auch für Dateien auf dem Server: Bilder, CSS und Skripte werden über die Serververbindung geladen
- PHP-Dateien mit hinterlegter Web-URL zeigen die Live-Seite und laden nach dem Speichern neu
- Zurück/Vor nach gefolgten Links, Neu laden
- Entwicklertools (F12-Konsole der Seite) und „Im Browser öffnen“ (Standardbrowser; unbenannte Dateien und Serverdateien ohne Web-URL als temporäre Kopie)

**KI-Agenten**
- Coding-Agenten im rechten Panel (Reiter „KI“, Strg+Umschalt+A): Claude Code, Codex (ChatGPT) und Gemini CLI über das Agent Client Protocol (ACP, wie in Zed); eigene ACP-Agenten lassen sich hinzufügen
- Eigener Agent ohne Node.js („In Nebula eingebaut“): mit API-Schlüssel für Anthropic (Claude) oder OpenAI, für jeden OpenAI-kompatiblen Dienst (OpenRouter, LM Studio …, Adresse unter „Server“) oder lokal mit Ollama. Er liest Dateien (auch ungespeicherte Tabs), listet und durchsucht das Projekt, ändert Dateien per Diff und führt Befehle in cmd.exe aus – Änderungen und Befehle erst nach Bestätigung. Das Modell wählt man in der Werkzeugleiste aus der Liste des Anbieters
- KI-Befehle auf der Auswahl (Rechtsklick im Editor › „KI“ oder Befehlspalette „KI: …“): Erklären, Kommentieren, Umschreiben… (mit eigener Anweisung) und Fehler beheben (gemeldete Probleme gehen mit); ohne Auswahl gilt die aktuelle Zeile. Der Auftrag geht an den Agenten im Panel, Änderungen kommen als Diff zum Übernehmen (auch bei Serverdateien)
- Der Agent arbeitet im geöffneten Projekt: Antworten als Markdown, Gedanken, Plan, Werkzeugaufrufe mit Diffs; vor Änderungen und Befehlen fragt er nach (Erlauben / Immer erlauben / Ablehnen). Änderungen landen rückgängig machbar im offenen Tab und werden gespeichert; mit „Beim Speichern hochladen“ gehen sie auch auf den Server
- Eingabe mit Werkzeugleiste: Agent, Modell, Aufwand und weitere Einstellungen des Agenten (Modell und Aufwand werden pro Agent gemerkt), Modus darunter; Stopp per ■ oder Esc
- Dateien anhängen: Büroklammer (aktuelle Datei, Dateiauswahl) oder Rechtsklick in der Seitenleiste „An KI-Agent anhängen“, auch für Serverdateien; Serverdateien kann der Agent auch ändern (über einen Pfad `\\nebula\…`, Nebula lädt die Änderung hoch)
- Verbleibendes 5-Stunden- und Wochenlimit des Claude- oder ChatGPT-Abos (Codex) unter der Eingabe (ohne Tokenverbrauch)
- Einstellungen → KI-Agenten: Status prüfen, Anmelden mit den Methoden des Anbieters (Browser-Login mit Claude-, ChatGPT- oder Google-Konto, oder API-Schlüssel in der Windows-Anmeldeinformationsverwaltung), Abmelden
- Node.js: ein installiertes Node wird genutzt, sonst lädt Nebula auf Wunsch die aktuelle LTS-Version (SHA-256-geprüft) nach `%APPDATA%\nebula\runtime`; die Agenten liegen in `%APPDATA%\nebula\agents`

**Erweiterungen (Nova-kompatibel)**
- Installation direkt aus Novas Extension Library (Suche, Kategorien, Beschreibung), aus einem Ordner oder einer ZIP-Datei
- JavaScript-Laufzeit (QuickJS) mit der Nova-Extension-API: Befehle, Einstellungen, Editor- und Dokument-API, Meldungen, Paletten, Benachrichtigungen, Prozesse (auch JSON-RPC), fetch, Dateisystem, Pfade, Zwischenablage, Anmeldedaten, Timer, Seitenleisten (TreeView)
- Language Server (LanguageClient): Probleme, Vervollständigung, Hover-Infos, Gehe zu Definition (Strg+Klick oder F12)
- Gehe zu Definition auch ohne Language Server: CSS-Klassen und IDs aus `class`/`id`-Attributen und Selektor-Strings in Skripten (CSS-, SCSS-, LESS-Dateien und `<style>`-Blöcke), Funktionen und Klassen in PHP, JavaScript/TypeScript und weiteren Sprachen, Dateipfade in Strings (`href`, `src`, `require`, `import`); bei mehreren Treffern eine Auswahlliste. Springt ein, wenn keine Erweiterung eine Definition liefert
- Formatierer über `onWillSave`, Prüf-Assistenten (Issues) mit Wellenlinien und Problemliste in der Statusleiste
- Einstellungsseiten werden aus den Manifesten der Erweiterungen erzeugt; Projekteinstellungen liegen wie bei Nova in `.nova/Configuration.json`
- Bildschirmleser (Sprachausgabe, NVDA): der Editor meldet sich als mehrzeiliges Textfeld mit dem Dateinamen, mit den sichtbaren Zeilen als Text und der Cursorposition/Auswahl darin
- Clips und Themes aus Erweiterungen (Themes erscheinen automatisch in der Theme-Auswahl); Erweiterungskonsole für Ausgaben und Fehler
- macOS-typische Aufrufe werden übersetzt (`/usr/bin/env`, Shell-Skripte über Git Bash, npm-Shims). Erweiterungen mit macOS-Programmen oder Tree-sitter-Grammatiken als `.dylib` laufen unter Windows nicht.

**Oberfläche**
- Oberfläche auf Deutsch, Englisch, Spanisch, Französisch oder Italienisch: automatisch nach der Windows-Anzeigesprache (Englisch, wenn es keine Übersetzung gibt) oder fest unter Einstellungen › Editor. Sprachdateien sind JSON (deutscher Text → Übersetzung); „Sprachdatei exportieren…“ liefert eine Vorlage mit allen Texten, „…importieren…“ legt sie in `%APPDATA%\nebula\languages` ab, wo sie eingebaute Sprachen ergänzen oder ersetzen. Fehlende Texte erscheinen auf Englisch
- Windows-Integration (Einstellungen › Windows), ohne Installer-Programm und ohne Adminrechte: „Installieren“ kopiert Nebula nach `%LOCALAPPDATA%\Programs\Nebula`, legt einen Startmenü-Eintrag an und trägt es unter „Apps“ zum Deinstallieren ein; „Mit Nebula öffnen“ im Explorer-Kontextmenü (Dateien, Ordner, Ordnerhintergrund); Nebula bei „Öffnen mit“ für Text- und Code-Dateien. Läuft Nebula schon, öffnen sich solche Dateien im vorhandenen Fenster (`--new-instance` erzwingt ein zweites)
- Einstellungen exportieren und importieren (Menü Einstellungen oder Befehlspalette), z. B. für einen neuen Rechner: Editor und Darstellung, eigene Themes, Sprachen und Snippets, Tastenkürzel, Server, KI-Agenten und Erweiterungen, einzeln wählbar, als eine JSON-Datei. Passwörter und API-Schlüssel kommen nicht mit; Server, Snippets und Agenten werden beim Import ergänzt, fehlende Erweiterungen aus der Bibliothek nachinstalliert
- Eigene Titelleiste mit Tabs; Tabs scrollen, lassen sich per Drag sortieren und haben ein Kontextmenü (Speichern, Duplizieren, Drucken, Schließen, andere/rechts/alle schließen, Pfad kopieren, im Explorer zeigen)
- Neue Datei mit Sprache: Rechtsklick (oder Umschalt+Klick) auf das „+“ in der Tableiste wählt Sprache und Dateiendung (z. B. „Ohne Titel 1.css“); „Speichern unter“ schlägt die Endung der eingestellten Sprache vor
- Mehrere Fenster: „Neues Fenster“ (Strg+Umschalt+N) oder „Ordner in neuem Fenster öffnen…“ (Menü Datei, Projekt-Umschalter; Strg+Klick auf ein zuletzt geöffnetes Projekt). Jedes Fenster hat sein eigenes Projekt; Einstellungen und Server sind gemeinsam – was ein Fenster ändert, übernehmen die anderen sofort, ohne dass sich Fenster gegenseitig etwas überschreiben
- Tabs anheften (Kontextmenü oder Befehlspalette): angeheftete Tabs stehen links, zeigen eine Nadel statt des Schließen-Knopfs (Klick löst sie) und bleiben bei „Andere schließen“, „Rechts davon schließen“, „Alle schließen“ und Mittelklick erhalten; der Zustand wird mit der Sitzung gespeichert
- Windows-11-Andocklayouts am Maximieren-Knopf, Mica-Hintergrund in der Seitenleiste, native Schatten und abgerundete Ecken
- Seitenleiste mit Dateien, Servern und Projektsuche; Git-Status (Branch, geänderte und neue Dateien)
- Zuletzt geöffnete Projekte: Klick auf den Projektnamen in der Seitenleiste, Menü, Startseite oder Befehlspalette. Pro Projekt merkt sich Nebula die offenen Dateien. Die Listen der zuletzt geöffneten Projekte und Dateien lassen sich auf der Startseite („Leeren“) und im Datei-Menü leeren.
- Befehlspalette mit allen Befehlen
- Themes „Nebula Dunkel“ und „Nebula Hell“ (Paletten nach Novas Themes), dazu „Nebula Kontrast Dunkel/Hell“ mit Text in WCAG-AAA-Kontrast (mindestens 7:1); ist unter Windows ein Kontrastdesign aktiv, wird das passende Kontrast-Theme automatisch genutzt (abschaltbar unter Einstellungen › Design). Eigene Themes lassen sich bearbeiten, als JSON importieren und exportieren, und Nova-Themes (.css) können importiert werden. Für helles und dunkles Design ist je ein Theme wählbar; umgeschaltet wird in den Einstellungen, im Menü „Ansicht → Erscheinungsbild“ oder per Knopf in der Titelleiste (Sonne, Mond oder automatisch wie Windows).
- Einstellungen für Editor, Design und Sprachen (Dateiendungen, Einrückung, eigene Vervollständigungen)
- Eigene Tastenkürzel (Einstellungen → Tastenkürzel): neue Kombination aufnehmen, weitere hinzufügen, auf Standard zurücksetzen; doppelt belegte Kürzel werden rot markiert

## Tastenkürzel

Alle Kürzel lassen sich unter Einstellungen → Tastenkürzel ändern. Standardbelegung:

| Aktion | Kürzel |
|---|---|
| Befehlspalette | Strg+Umschalt+P |
| Neu / Öffnen / Ordner öffnen / Schnell öffnen | Strg+N / Strg+O / Strg+Umschalt+O / Strg+P |
| Neues Fenster | Strg+Umschalt+N |
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
| Gehe zu Definition | F12 / Strg+Klick |
| Makro aufnehmen/beenden, abspielen | Strg+Umschalt+R, Strg+Umschalt+M |
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
  agent/               Eingebauter Agent: ACP im Thread, Anthropic-/OpenAI-API (llm.rs), Werkzeuge (tools.rs)
  mcp.rs               Nebula als MCP-Server (HTTP, Werkzeuge in app/mcp_tools.rs)
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
  i18n.rs, i18n/       Übersetzungen: t!("deutscher Text"), Sprachdateien en/es/fr/it.json
  win32.rs             Mica, Andocklayouts, Datei-Zwischenablage, Cursorposition, Browser öffnen, Dateien hinausziehen
  ui/                  Titelleiste, Seitenleiste, Projektsuche, Git, Icons
  editor/              Dokument, Befehle, Vervollständigung, Emmet, Faltung, Suche, Widget
  highlight/           tree-sitter-Highlighting im Hintergrund (Grammatiken werden bei Bedarf geladen);
                       große Dateien inkrementell (incremental.rs)
  perf.rs              Zeitmessung pro Frame (NEBULA_PERF=1) und Benchmark-Test
  queries/             Eigene Abfragen für HTML und CSS mit Nova-Kategorien
  remote/              FTP/FTPS (suppaftp) und SFTP (russh), Worker pro Server, SSH-Shell (terminal.rs, vt100)
  theme.rs             Themes, Theme-Dateien
  theme_import.rs      Import von Nova-Themes
  settings.rs          Einstellungen, Server, Sprachen, Veröffentlichen
```
