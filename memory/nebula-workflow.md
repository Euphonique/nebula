---
name: nebula-workflow
description: Wie der Nutzer an Nebula arbeiten lassen will und bewährte Test-/Bearbeitungstricks in dieser Umgebung
metadata:
  node_type: memory
  type: feedback
  originSessionId: c291b96a-8195-4b92-89fb-763a4eb5e64b
  modified: 2026-10-18T18:12:43.757Z
---

- Ein Git-Commit pro Aufgabe (Nutzer ausdrücklich: „Ja, pro Aufgabe“). Auf Deutsch kommunizieren, UI-Texte deutsch.
- Jede Funktion in der App per Screenshot prüfen: isolierte Konfiguration mit `NEBULA_CONFIG_DIR=<scratchpad>\cfg`, Skripte `shot.ps1`/`click.ps1` im Scratchpad (click.ps1 bricht ab, wenn Nebula nicht im Vordergrund ist – nie in fremde Fenster klicken). Nach SendKeys mit Strg/Alt die Modifier per keybd_event loslassen, sonst kommen Tasten als ^C an.
- Kein Python vorhanden. perl/sed-Ersetzungen mit `|` im Muster zerstören Dateien – für Rust-Code mit Closures das Edit-Tool nehmen.
- A: ist ein Netzlaufwerk (\\Diskstation\home). `canonicalize()` und `git rev-parse --show-toplevel` liefern dort UNC-Pfade, die nicht zu `A:\…` passen – hat schon zweimal Fehler verursacht, bei Pfadvergleichen daran denken. Nebula zum Testen mit dangerouslyDisableSandbox starten, sonst sieht es die Dateien nicht. Fenster per MoveWindow auf 1800x1100 setzen (sonst riesiger Ultrawide-Screenshot), vorher Alt-Taste drücken, damit SetForegroundWindow klappt. Testskripte (shot.ps1, click.ps1) liegen im Scratchpad früherer Sitzungen (c291b96a, 54481c14) und müssen ins neue Scratchpad kopiert werden. Lässt Windows Nebula nicht in den Vordergrund (Nutzer arbeitet gerade), nicht erzwingen, sondern mit automatischen Tests prüfen. Der Debug-Start dauert teils >5 s – auf das Fenster warten.
- Server-UI-Tests: `NEBULA_SFTP_ROOT=… cargo test serve_sftp -- --ignored` (127.0.0.1:2222, test/secret, mit Echo-Shell); Passwort per `NEBULA_SITE=id:user NEBULA_PASS=secret cargo test set_site_password -- --ignored` setzen statt über die UI, danach mit leerem NEBULA_PASS wieder löschen. Der Nutzer hat einen echten Eintrag „…:hosting204954“ – nie anfassen.
- Git-Funktionen nie im Nebula-Repo testen (keine Test-Commits dort), sondern im Scratchpad ein Wegwerf-Repo plus `git init --bare remote.git` als lokalen „Server“ anlegen.
- Läuft der SFTP-Testserver (`cargo test serve_sftp`), sperrt er das Test-Binary: vor `cargo test` den Prozess auf Port 2222 beenden.
- Popup-ID-Kollision: `context_menu` und `Popup::menu` auf derselben Response teilen sich die ID → eigene `.id(...)` setzen. Dropdowns im Einstellungs-Modal als `egui::Area` mit `Order::Tooltip` (liegt über dem Modal, bekommt Tastatur).
- Testsitzung zeigt Startseite erst, wenn in cfgsettings.json `session`/`project_tabs` geleert und `cfgackup` gelöscht ist; `launch.ps1` im Scratchpad startet mit isolierter Konfiguration.
- egui-Fallstricke: Popups außerhalb eines Modals (z. B. Einstellungen) bekommen keine Tastatureingaben → Inhalte im Modal selbst zeigen; die Einstellungsseiten bekommen ihren Zustand als `state`-Parameter (self.settings_ui ist währenddessen herausgenommen); `ui.horizontal` in ScrollAreas zentriert vertikal und verschiebt Inhalte → `Layout::left_to_right(Align::Min)`.
- Vor `cargo build` laufende nebula.exe beenden (sonst „Zugriff verweigert“); `cfg\backup` danach löschen.
- Der Nutzer hat eine Tastatur, auf der F-Tasten Sonderfunktionen haben (Fn+F2 = echtes F2): F-Tasten-Kürzel immer mit Strg/Alt-Alternative anbieten.

**Why:** Diese Punkte haben in früheren Sitzungen Zeit gekostet oder wurden vom Nutzer so gewünscht.
**How to apply:** Bei jeder weiteren Arbeit an [[nebula-progress]] beachten.
