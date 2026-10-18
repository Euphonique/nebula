# Nebula – offene Ideen

Priorität: **A** = fehlt spürbar im Alltag, **B** = deutlicher Komfortgewinn, **C** = nice to have.

## Sicherheit und Datenerhalt

- **C** `.bak`-Kopie auf dem Server vor dem Überschreiben (eine lokale Kopie leistet schon der lokale Verlauf).

## Editor

- **B** Eigene Snippets pro Sprache in den Einstellungen anlegen/bearbeiten (mit Platzhaltern `$1`, `${2:name}`).
- **C** Makros aufnehmen und abspielen.
- **C** Rechtschreibprüfung für Texte, Kommentare und Markdown (Windows-Rechtschreibprüfung nutzen).
- **C** Ligaturen in der Editorschrift (egui formt keine Ligaturen; bräuchte eigenes Text-Shaping).

## Suchen und Navigieren

- **C** Regex-Ersetzung mit Vorschau.
- **B** „Schnell öffnen“ (Strg+P) auch für Dateien auf dem Server (aus bereits geladenen Ordnern) und mit `:Zeile`-Suffix.
- **C** Gehe zu Definition ohne Language Server für einfache Fälle (CSS-Klassen, PHP-Funktionen im Projekt).

## Git

- **C** Blame für die aktuelle Zeile (Autor/Datum dezent am Zeilenende).

## Server und Veröffentlichen

- **B** Synchronisieren in beide Richtungen mit Vorschau (neu/geändert/gelöscht) und Ausschlussliste (`.git`, `node_modules`, `.env`).
- **B** Server in Ordnern gruppieren, Server duplizieren, Import aus FileZilla (`sitemanager.xml`).
- **C** Suche im Terminal-Verlauf (Strg+F im Terminal, Treffer im Scrollback hervorheben).
- **C** Datenbank light: MySQL über SSH-Tunnel, Tabellen ansehen, Abfragen ausführen.
- **C** Serverdateien zwischenspeichern, damit sie offline lesbar bleiben.

## Live-Vorschau und Web

- **B** Vorschau in verschiedenen Gerätebreiten (Handy/Tablet/Desktop) umschaltbar.
- **B** Lokaler PHP-Server für die Vorschau (wenn `php.exe` vorhanden), statt nur Live-URL.

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

Runde 1 und 2 sind abgeschlossen.

1. KI im rechten Panel (Agenten über ACP, eigener Chat).
2. Danach weitere B-Punkte aus der Liste oben.


## Erledigt

Externe Änderungen erkennen, Wiederherstellung nach Absturz, geteilte Ansicht, Blockauswahl, Formatieren beim Speichern und `.editorconfig`, zuletzt geöffnete Projekte, Git-Markierungen, Unterschiede im Konfliktdialog, eigene Tastenkürzel.

Danach: Knöpfe für geteilte Ansicht und rechtes Panel, rechtes Panel mit Vorschau und Gliederung, lokales Terminal (PowerShell, Git Bash, cmd), Wort unter dem Cursor hervorheben, passendes HTML-Tag hervorheben und mit umbenennen, Gehe zu Symbol (Strg+R), Navigationsverlauf (Alt+←/→), Lesezeichen, Textwerkzeuge, Suchen/Ersetzen in der Auswahl, Markierungen am rechten Rand, klickbare Pfadleiste (auch Netzlaufwerke werden jetzt korrekt geöffnet), Projektsuche mit Dateimustern und `.gitignore`, Projektsuche auf dem Server über SSH (abbrechbar), automatisch speichern, lokaler Verlauf, neues Fensterlayout mit Logo, Vergleich als Tab (Git-Markierungen funktionieren jetzt auch auf Netzlaufwerken), Git-Tab mit Commit, Pull/Push und Branches, Übertragungsliste (Neuverbindung nach Verbindungsabbruch repariert), gespeicherte Serverbefehle, Zen-Modus, Schriftart und Zeilenhöhe, Farbwähler für CSS-Farben, Bildvorschau, Minimap.
