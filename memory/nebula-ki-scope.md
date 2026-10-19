---
name: nebula-ki-scope
description: "KI-Integration in Nebula – abgestimmter Umfang, getroffene Entscheidungen, Aufbau im Code und was noch offen ist"
metadata:
  node_type: memory
  type: project
  originSessionId: 10a7e079-fb2f-49fa-8901-631ac611494f
  modified: 2026-10-19T21:14:55.097Z
---

Abgestimmt am 2026-10-19: zuerst Agenten über ACP (wie Zed), danach eigener Agent/Chat für API-Schlüssel (Anthropic, OpenAI inkl. kompatibler Basis-URL) und Ollama, dann KI-Befehle auf der Auswahl, Commit-Nachricht im Git-Tab, Nebula als MCP-Server, Inline-Vorschläge (unbedingt abschaltbar, Kosten).

Entscheidungen:
- Bei Node bleiben: Direkter Abo-OAuth (wie im Projekt A:\Development\AntiSpam, ClaudeOAuthPlugin) liefert bei Anthropic nur Haiku und ist seit 02/2026 untersagt; der Nutzer will die besseren Modelle. ChatGPT-Abo-OAuth wäre technisch/regelseitig eher möglich.
- Alles liegt in %APPDATA%\nebula: config\, runtime\node\ (Nebula lädt Node-LTS bei Bedarf, SHA-256-geprüft), agents\ (npm_config_cache für npx). Installiertes Node wird mitbenutzt, Nebulas eigenes hat Vorrang.
- Server-Bearbeitung durch den Agenten: Weg A (lokales Projekt + „Beim Speichern hochladen“) umgesetzt; virtuelle Serverpfade (B) offen.
- UI: Eingabe als Karte mit Chip-Werkzeugleiste (Agent, Modell, Aufwand, ⋯), Modus und Limits darunter; Reiter rechts: Gliederung, KI, Vorschau. Englische Werte der Agenten werden übersetzt (choice_name in src/app/agent.rs).

Code: src/acp/ (mod.rs Verbindung + Presets, session.rs Sitzung/configOptions/Anhänge/Login, node.rs Node-Verwaltung, usage.rs Limits über `/usage` in unsichtbarer Sitzung – nur claude-agent-acp, kostet keine Tokens), src/app/agent.rs (Panel), src/app/agents_page.rs (Einstellungen). Tests mit Schein-Agent in src/acp/tests.rs; Netz-Tests `download_node`, `replace_node_in_use` (ignored).

Prüfen ohne Kosten: initialize + session/new + `/usage` + set_config_option verbrauchen kein Kontingent; echte Prompts an Claude/Codex/Gemini nur nach Rückfrage. Codex meldet auch ohne Login „bereit“.

**Why:** Nutzer hatte KI bewusst zurückgestellt und den Umfang vorher festgelegt; die Entscheidungen oben wurden ausführlich besprochen.
**How to apply:** Offene Punkte in todo.md (Abschnitt KI) in der Reihenfolge abarbeiten; siehe [[nebula-progress]], [[nebula-workflow]].
