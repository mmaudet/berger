// Berger: open-source email triage daemon.
// Copyright (C) 2026 Michel-Marie Maudet
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The WebUI's single static stylesheet, embedded in the binary.
//!
//! A small hand-written sheet (CLAUDE.md §4.3 keeps the WebUI dependency-light)
//! with a sober neutral palette in the spirit the PRD §5.7 asks for.

/// The stylesheet served at `/static/berger.css`.
pub const STYLESHEET: &str = r#":root {
  --bg: #f7f7f8;
  --surface: #ffffff;
  --border: #e3e3e6;
  --text: #1c1c1f;
  --text-muted: #6b6b73;
  --accent: #3b5bdb;
  --ok: #2f7d4f;
  --warn: #b45309;
  --err: #b42318;
  --radius: 8px;
}

* { box-sizing: border-box; }

html, body {
  margin: 0;
  padding: 0;
  background: var(--bg);
  color: var(--text);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
    Helvetica, Arial, sans-serif;
  font-size: 14px;
  line-height: 1.5;
}

a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }

.layout {
  max-width: 960px;
  margin: 0 auto;
  padding: 0 24px 64px;
}

header.topbar {
  border-bottom: 1px solid var(--border);
  background: var(--surface);
  margin-bottom: 32px;
}

.topbar-inner {
  max-width: 960px;
  margin: 0 auto;
  padding: 16px 24px;
  display: flex;
  align-items: baseline;
  gap: 24px;
}

.brand {
  font-weight: 600;
  font-size: 16px;
  color: var(--text);
}

.brand:hover { text-decoration: none; }

nav.tabs { display: flex; gap: 18px; }

nav.tabs a {
  color: var(--text-muted);
  padding-bottom: 2px;
}

nav.tabs a.active {
  color: var(--text);
  border-bottom: 2px solid var(--accent);
}

h1 {
  font-size: 20px;
  font-weight: 600;
  margin: 0 0 4px;
}

h2 {
  font-size: 15px;
  font-weight: 600;
  margin: 28px 0 10px;
}

p.subtitle {
  color: var(--text-muted);
  margin: 0 0 24px;
}

.cards {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  gap: 14px;
}

.card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 16px;
}

.card .label {
  color: var(--text-muted);
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.card .value {
  font-size: 24px;
  font-weight: 600;
  margin-top: 6px;
}

.card .note {
  color: var(--text-muted);
  font-size: 12px;
  margin-top: 4px;
}

table {
  width: 100%;
  border-collapse: collapse;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}

th, td {
  text-align: left;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border);
  vertical-align: top;
}

th {
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-muted);
  background: #fafafa;
}

tr:last-child td { border-bottom: none; }

.muted { color: var(--text-muted); }

.tag {
  display: inline-block;
  background: #eef0fb;
  color: var(--accent);
  border-radius: 4px;
  padding: 1px 7px;
  font-size: 12px;
  margin: 1px 3px 1px 0;
}

.pill {
  display: inline-block;
  border-radius: 4px;
  padding: 1px 7px;
  font-size: 12px;
}

.pill.ok { background: #e7f3ec; color: var(--ok); }
.pill.err { background: #fbeae8; color: var(--err); }

.section {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 16px 18px;
  margin-bottom: 16px;
}

.kv {
  display: grid;
  grid-template-columns: 140px 1fr;
  gap: 6px 16px;
}

.kv dt { color: var(--text-muted); }
.kv dd { margin: 0; }

pre {
  background: #1c1c1f;
  color: #e8e8ea;
  border-radius: var(--radius);
  padding: 14px;
  overflow-x: auto;
  font-size: 12.5px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 6px 0 0;
}

code {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

.empty {
  color: var(--text-muted);
  background: var(--surface);
  border: 1px dashed var(--border);
  border-radius: var(--radius);
  padding: 20px;
  text-align: center;
}

footer.site {
  color: var(--text-muted);
  font-size: 12px;
  margin-top: 40px;
  border-top: 1px solid var(--border);
  padding-top: 16px;
}
"#;
