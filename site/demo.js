// Ссылка на репозиторий. На <owner>.github.io/<repo>/ она определяется сама;
// для своего домена впишите её сюда, например "https://github.com/owner/comma_editor".
const REPO = null;

(function repoLinks() {
  let base = REPO;
  const host = location.hostname;
  if (!base && host.endsWith(".github.io")) {
    const owner = host.slice(0, -".github.io".length);
    const first = location.pathname.split("/").filter(Boolean)[0];
    base = `https://github.com/${owner}/${first || host}`;
  }
  if (!base) return;
  for (const a of document.querySelectorAll("a[data-repo]")) a.href = base + a.dataset.repo;
})();

// Демо: тот же people.csv, что лежит в samples/.
const SRC =
  'id,name,email,city,signup_date,balance\n' +
  '1,Alice Johnson,alice@example.com,London,2024-01-15,1250.50\n' +
  '2,"Smith, Bob",bob@example.com,"New York",2024-02-03,980.00\n' +
  '3,Clara Diaz,clara@example.com,Madrid,2024-02-28,0\n' +
  '4,Dmitry Petrov,dmitry@example.com,Moscow,2024-03-11,15000.75\n' +
  '5,Emma Brown,emma@example.com,"Line one\nline two",2024-04-01,42\n' +
  '6,"Quote ""Q"" Lee",q@example.com,Seoul,2024-04-19,7.25\n';

// Разбор с сохранением исходных байтов каждого поля, как в ядре.
function parse(src) {
  const records = [];
  let fields = [];
  let i = 0;
  while (i < src.length) {
    const start = i;
    let value = "";
    if (src[i] === '"') {
      i++;
      while (i < src.length) {
        if (src[i] === '"') {
          if (src[i + 1] === '"') { value += '"'; i += 2; } else { i++; break; }
        } else value += src[i++];
      }
    }
    while (i < src.length && src[i] !== "," && src[i] !== "\n" && src[i] !== "\r") value += src[i++];
    fields.push({ raw: src.slice(start, i), value });
    if (src[i] === ",") { i++; continue; }
    let eol = "";
    if (src[i] === "\r") eol += src[i++];
    if (src[i] === "\n") eol += src[i++];
    records.push({ fields, eol });
    fields = [];
  }
  return records;
}

const quote = (v) => (/[",\r\n]/.test(v) ? '"' + v.replace(/"/g, '""') + '"' : v);
const esc = (s) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);
const bytes = (s) => new TextEncoder().encode(s).length;

const original = parse(SRC);
let values = original.map((r) => r.fields.map((f) => f.value));
let mode = "comma";

// Что будет записано в поле: исходные байты, если значение не трогали.
function written(r, c) {
  const f = original[r].fields[c];
  const v = values[r][c];
  if (mode === "comma" && v === f.value) return f.raw;
  return quote(v);
}

const sheet = document.getElementById("sheet");
const raw = document.getElementById("raw");
const verdict = document.getElementById("verdict");
const resetBtn = document.getElementById("reset");

function cellHTML(v) {
  return esc(v).replace(/\n/g, '<span class="nl">↵</span>');
}

function renderSheet() {
  const head = original[0].fields.map((f) => `<th>${esc(f.value)}</th>`).join("");
  let html = `<thead><tr><th class="rn"></th>${head}</tr></thead><tbody>`;
  for (let r = 1; r < original.length; r++) {
    html += `<tr><td class="rn">${r}</td>`;
    values[r].forEach((v, c) => {
      const cls = ["cell"];
      if (/^-?\d+([.,]\d+)?$/.test(v)) cls.push("num");
      if (v !== original[r].fields[c].value) cls.push("edited");
      html += `<td class="${cls.join(" ")}" data-r="${r}" data-c="${c}">${cellHTML(v)}</td>`;
    });
    html += "</tr>";
  }
  sheet.innerHTML = html + "</tbody>";
}

function renderRaw() {
  let html = "";
  let changedFields = 0;
  let changedRecords = 0;
  let newBytes = 0;
  let out = "";
  original.forEach((rec, r) => {
    let line = "";
    let mod = false;
    rec.fields.forEach((f, c) => {
      const w = written(r, c);
      out += (c ? "," : "") + w;
      if (c) line += ",";
      if (w === f.raw) {
        line += esc(f.raw);
      } else {
        mod = true;
        if (values[r][c] !== f.value) changedFields++;
        newBytes += bytes(w);
        line += (f.raw ? `<del>${esc(f.raw)}</del>` : "") + `<mark>${esc(w)}</mark>`;
      }
    });
    out += rec.eol;
    if (mod) changedRecords++;
    const eol = rec.eol === "\r\n" ? "␍␊" : "␊";
    html += `<div class="rec ${mod ? "mod" : "same"}"><span class="g">${r + 1}</span>` +
      `<span class="t">${line}<span class="eol">${eol}</span></span></div>`;
  });
  raw.innerHTML = html;

  const total = bytes(out);
  const edits = values.some((row, r) => row.some((v, c) => v !== original[r].fields[c].value));
  resetBtn.disabled = !edits;

  if (out === SRC) {
    verdict.innerHTML = `<b>Файл совпадает с исходным байт в байт</b> — ${total} байт.`;
  } else if (mode === "comma") {
    verdict.innerHTML =
      `<b>Изменено полей: ${changedFields}.</b> Остальные ${total - newBytes} из ${total} байт ` +
      `скопированы из исходного файла как есть.`;
  } else {
    verdict.innerHTML =
      `<b>Изменились строки: ${changedRecords}</b>, хотя правок ${edits ? "было " + changedFields : "не было"}. ` +
      `Запись заново по правилам CSV убирает «лишние» кавычки, и в диффе появляется то, чего вы не трогали.`;
  }
}

function render() { renderSheet(); renderRaw(); }

function edit(td) {
  if (!td || td.classList.contains("editing")) return;
  const r = +td.dataset.r;
  const c = +td.dataset.c;
  const v = values[r][c];
  const multi = v.includes("\n");
  const input = document.createElement(multi ? "textarea" : "input");
  input.value = v;
  input.spellcheck = false;
  if (multi) input.rows = v.split("\n").length;
  td.classList.add("editing");
  td.textContent = "";
  td.appendChild(input);
  input.focus();
  input.select();

  let done = false;
  const finish = (commit, move) => {
    if (done) return;
    done = true;
    if (commit) values[r][c] = input.value;
    render();
    if (move) {
      const next = sheet.querySelector(`td[data-r="${r + move[0]}"][data-c="${c + move[1]}"]`);
      if (next) edit(next);
    }
  };
  input.addEventListener("blur", () => finish(true));
  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.preventDefault(); finish(false); }
    else if (e.key === "Enter" && !(multi && (e.altKey || e.shiftKey))) {
      e.preventDefault(); finish(true, [e.shiftKey ? -1 : 1, 0]);
    } else if (e.key === "Tab") {
      e.preventDefault(); finish(true, [0, e.shiftKey ? -1 : 1]);
    }
  });
}

// Демо есть только на главной странице.
if (sheet) {
  sheet.addEventListener("click", (e) => edit(e.target.closest("td.cell")));

  for (const b of document.querySelectorAll(".disk .seg button")) {
    b.addEventListener("click", () => {
      mode = b.dataset.mode;
      for (const o of document.querySelectorAll(".disk .seg button")) o.setAttribute("aria-selected", o === b);
      renderRaw();
    });
  }

  resetBtn.addEventListener("click", () => {
    values = original.map((r) => r.fields.map((f) => f.value));
    render();
  });

  render();
}

// Скриншоты: вкладки.
(function shots() {
  const tabs = [...document.querySelectorAll(".shots-tabs [role=tab]")];
  for (const tab of tabs) {
    tab.addEventListener("click", () => {
      for (const other of tabs) {
        const selected = other === tab;
        other.setAttribute("aria-selected", String(selected));
        document.getElementById(other.getAttribute("aria-controls")).hidden = !selected;
      }
    });
  }
})();
