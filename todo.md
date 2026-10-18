# Nebula – offene Ideen

Priorität: **A** = fehlt spürbar im Alltag, **B** = deutlicher Komfortgewinn, **C** = nice to have.

## Sicherheit und Datenerhalt

- **B** Automatisches Speichern (optional: bei Fokusverlust oder nach x Sekunden), für Serverdateien eher aus.
- **B** Lokaler Verlauf: frühere Fassungen einer Datei (z. B. die letzten 20 Speicherstände) ansehen, vergleichen und wiederherstellen – besonders wertvoll beim direkten Arbeiten auf dem Server.
- **B** Backup vor dem Überschreiben auf dem Server (optional `.bak` oder lokale Kopie).

## Editor

- **C** Minimap (verkleinerte Code-Ansicht). Markierungen in der Scrollleiste sind erledigt.
- **B** Eigene Snippets pro Sprache in den Einstellungen anlegen/bearbeiten (mit Platzhaltern `$1`, `${2:name}`).
- **C** Makros aufnehmen und abspielen.
- **C** Rechtschreibprüfung für Texte, Kommentare und Markdown (Windows-Rechtschreibprüfung nutzen).
- **C** Zen-/Fokusmodus (nur Editor, Vollbild).
- **C** Schriftart wählbar (inkl. Ligaturen), Zeilenhöhe einstellbar.

## Suchen und Navigieren

- **C** Regex-Ersetzung mit Vorschau.
- **B** „Schnell öffnen“ (Strg+P) auch für Dateien auf dem Server (aus bereits geladenen Ordnern) und mit `:Zeile`-Suffix.
- **C** Gehe zu Definition ohne Language Server für einfache Fälle (CSS-Klassen, PHP-Funktionen im Projekt).

## Git

- **B** Diff-Ansicht als eigener Tab: Arbeitsstand gegen letzten Commit, zwei beliebige Dateien vergleichen (auch lokal ↔ Server). Der Diff-Algorithmus (`src/diff.rs`) ist schon da.
- **B** Einfache Git-Aktionen: Dateien stagen, Commit mit Nachricht, Pull/Push, Branch wechseln.
- **C** Blame für die aktuelle Zeile (Autor/Datum dezent am Zeilenende).

## Server und Veröffentlichen

- **B** Synchronisieren in beide Richtungen mit Vorschau (neu/geändert/gelöscht) und Ausschlussliste (`.git`, `node_modules`, `.env`).
- **B** Übertragungsliste: laufende/fertige/fehlgeschlagene Übertragungen mit Fortschritt, Abbrechen und Wiederholen.
- **B** Server in Ordnern gruppieren, Server duplizieren, Import aus FileZilla (`sitemanager.xml`).
- **B** SSH-Terminal: gespeicherte Befehle pro Server per Klick (Cache leeren, `git pull`, Deploy), Suche im Terminal-Verlauf.
- **C** Datenbank light: MySQL über SSH-Tunnel, Tabellen ansehen, Abfragen ausführen.
- **C** Serverdateien zwischenspeichern, damit sie offline lesbar bleiben.

## Live-Vorschau und Web

- **B** Vorschau in verschiedenen Gerätebreiten (Handy/Tablet/Desktop) umschaltbar.
- **B** Lokaler PHP-Server für die Vorschau (wenn `php.exe` vorhanden), statt nur Live-URL.
- **C** Farbwähler beim Klick auf die Farbvorschau unter CSS-Farbwerten.
- **C** Bildvorschau: Bilder (PNG, JPG, SVG, WebP) in einem Tab anzeigen statt „keine Textdatei“, mit Größe/Abmessungen.

## KI-Unterstützung (wie in VS Code)

- **B** Agenten über das Agent Client Protocol (ACP, wie im Editor Zed) anbinden: Claude Code & Co. als Chat im rechten Panel, Änderungen kommen als Diff zum Annehmen/Ablehnen (Diff-Anzeige ist schon da), Rückfragen vor Dateizugriff/Befehlen.
- **B** Eigener KI-Chat als Reiter „KI“ im rechten Panel: Anbieter wählbar (Anthropic, OpenAI, lokal über Ollama), API-Schlüssel in der Windows-Anmeldeinformationsverwaltung. Kontext per Knopf: aktuelle Datei, Auswahl, Fehlerliste, ausgewählte Projektdateien.
- **B** KI-Befehle auf der Auswahl: Erklären, Kommentieren, Umschreiben/Refaktorisieren, Fehler beheben, Commit-Nachricht aus dem Diff – Ergebnis als Diff zum Übernehmen.
- **C** Inline-Vorschläge beim Tippen (graue Vervollständigung, Tab übernimmt).
- **C** Nebula als MCP-Server: externe Agenten sehen offene Dateien, Auswahl und Fehler und können Dateien im Editor öffnen.

## Oberfläche und Allgemein

- **B** Tabs anheften; mehrere Fenster/Projekte gleichzeitig.
- **B** Große Dateien (> 64 MB) schreibgeschützt öffnen statt ablehnen; Hex-Ansicht für Binärdateien.
- **B** Einstellungen exportieren/importieren (Server ohne Passwörter, Themes, Tastenkürzel).
- **B** Update-Hinweis, Installer und Eintrag „Mit Nebula öffnen“ im Explorer-Kontextmenü, Dateizuordnungen.
- **C** Oberfläche auch auf Englisch (Texte in Übersetzungsdateien auslagern).
- **C** Barrierefreiheit: Bildschirmleser im Editor (AccessKit ist schon eingebunden), hoher Kontrast.

## Qualität und Technik

- **B** Performance mit sehr großen Dateien (10–50 MB, minifiziertes JS in einer Zeile) und vielen Tabs prüfen; Highlighting inkrementell statt komplett neu.
- **B** Absturzbericht: Panics in eine Logdatei schreiben und beim nächsten Start anbieten.
- **C** Automatisierte UI-Tests für die wichtigsten Abläufe (Öffnen, Bearbeiten, Speichern lokal/Server).

## Nächste Schritte

1. Runde 2 (Arbeitsfluss): automatisch speichern und lokaler Verlauf, Diff als Tab, Git-Aktionen, Übertragungsliste, gespeicherte Serverbefehle.
2. Danach KI im rechten Panel (Agenten über ACP, eigener Chat).

## Erledigt

Externe Änderungen erkennen, Wiederherstellung nach Absturz, geteilte Ansicht, Blockauswahl, Formatieren beim Speichern und `.editorconfig`, zuletzt geöffnete Projekte, Git-Markierungen, Unterschiede im Konfliktdialog, eigene Tastenkürzel.

Danach: Knöpfe für geteilte Ansicht und rechtes Panel, rechtes Panel mit Vorschau und Gliederung, lokales Terminal (PowerShell, Git Bash, cmd), Wort unter dem Cursor hervorheben, passendes HTML-Tag hervorheben und mit umbenennen, Gehe zu Symbol (Strg+R), Navigationsverlauf (Alt+←/→), Lesezeichen, Textwerkzeuge, Suchen/Ersetzen in der Auswahl, Markierungen am rechten Rand, klickbare Pfadleiste (auch Netzlaufwerke werden jetzt korrekt geöffnet), Projektsuche mit Dateimustern und `.gitignore`, Projektsuche auf dem Server über SSH.
