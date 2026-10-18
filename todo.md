# Nebula – offene Ideen

Priorität: **A** = fehlt spürbar im Alltag, **B** = deutlicher Komfortgewinn, **C** = nice to have.

## Sicherheit und Datenerhalt

- **B** Automatisches Speichern (optional: bei Fokusverlust oder nach x Sekunden), für Serverdateien eher aus.
- **B** Lokaler Verlauf: frühere Fassungen einer Datei (z. B. die letzten 20 Speicherstände) ansehen, vergleichen und wiederherstellen – besonders wertvoll beim direkten Arbeiten auf dem Server.
- **B** Backup vor dem Überschreiben auf dem Server (optional `.bak` oder lokale Kopie).

## Editor

- **B** Minimap bzw. Markierungen in der Scrollleiste (Suchtreffer, Fehler, geänderte Zeilen, Cursor).
- **B** Textwerkzeuge: Zeilen sortieren, Duplikate entfernen, GROSS/klein/Titel, Zeilen verbinden, Auswahl umschließen (Klammern/Anführungszeichen/Tag um markierten Text), Zeilen umkehren.
- **B** Lesezeichen (Strg+F2 setzen, F2 springen) mit Markierung in der Leiste.
- **B** Gehe zu Symbol als Schnellsuche (Strg+R / Strg+Umschalt+O wie in VS Code) – die Symbolliste der Gliederung (`editor/symbols.rs`) ist schon da.
- **B** Klickbare Pfadleiste: Klick auf ein Element zeigt Geschwister zum Springen (wie Nova).
- **B** Passendes HTML-Tag hervorheben und beim Umbenennen des öffnenden Tags das schließende mit ändern.
- **B** Wort unter dem Cursor überall im Dokument dezent hervorheben.
- **B** Eigene Snippets pro Sprache in den Einstellungen anlegen/bearbeiten (mit Platzhaltern `$1`, `${2:name}`).
- **C** Makros aufnehmen und abspielen.
- **C** Rechtschreibprüfung für Texte, Kommentare und Markdown (Windows-Rechtschreibprüfung nutzen).
- **C** Zen-/Fokusmodus (nur Editor, Vollbild).
- **C** Schriftart wählbar (inkl. Ligaturen), Zeilenhöhe einstellbar.

## Suchen und Navigieren

- **B** Projektsuche mit Ein-/Ausschlussmustern (`*.php`, `!vendor/**`) und Berücksichtigung der `.gitignore`.
- **B** Suchen/Ersetzen nur in der Auswahl; Regex-Ersetzung mit Vorschau.
- **B** Navigationsverlauf: Zurück/Vor zur letzten Cursorposition (Alt+←/→), auch über Dateien hinweg.
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
- **B** SSH-Terminal: gespeicherte Befehle pro Server per Klick (Cache leeren, `git pull`, Deploy), Suche im Terminal-Verlauf, lokales Terminal (PowerShell) im selben Panel.
- **C** Datenbank light: MySQL über SSH-Tunnel, Tabellen ansehen, Abfragen ausführen.
- **C** Serverdateien zwischenspeichern, damit sie offline lesbar bleiben.

## Live-Vorschau und Web

- **B** Vorschau in verschiedenen Gerätebreiten (Handy/Tablet/Desktop) umschaltbar.
- **B** Lokaler PHP-Server für die Vorschau (wenn `php.exe` vorhanden), statt nur Live-URL.
- **C** Farbwähler beim Klick auf die Farbvorschau unter CSS-Farbwerten.
- **C** Bildvorschau: Bilder (PNG, JPG, SVG, WebP) in einem Tab anzeigen statt „keine Textdatei“, mit Größe/Abmessungen.

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

## Erledigt (A-Prioritäten)

Externe Änderungen erkennen, Wiederherstellung nach Absturz, geteilte Ansicht, Blockauswahl, Formatieren beim Speichern und `.editorconfig`, zuletzt geöffnete Projekte, Git-Markierungen, Unterschiede im Konfliktdialog, eigene Tastenkürzel.
