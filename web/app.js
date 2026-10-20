import init, { WebEditor } from './pkg/openfuse_desktop.js';
const $ = (id) => document.getElementById(id);
let editor, filename = 'OpenFUSE preset.fuse', busy = false;
let dark = matchMedia('(prefers-color-scheme: dark)').matches;
const error = (e) => { $('error').textContent = String(e); };
const status = (message) => { $('status').textContent = message; };
const discard = () => !editor.dirty() || confirm('Replace this preset? Download your edits first if you want to keep them.');
function theme() {
  document.body.classList.toggle('dark', dark);
  $('theme').textContent = dark ? 'Light' : 'Dark';
  editor?.appearance(dark);
}
function action(fn) { return () => { try { $('error').textContent = ''; fn(); } catch (e) { error(e); } }; }
function download() {
  const xml = editor.export_xml();
  const url = URL.createObjectURL(new Blob([xml], { type: 'application/xml;charset=utf-8' }));
  const link = document.createElement('a');
  link.href = url; link.download = filename; document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
  status('Download requested. Check your downloads before closing.');
}
async function openFile(file) {
  if (!file || busy) return;
  if (file.size > 1024 * 1024) throw new Error('Preset exceeds the 1 MiB browser import limit.');
  busy = true;
  try {
    const xml = new TextDecoder('utf-8', { fatal: true }).decode(await file.arrayBuffer());
    // The Rust parser validates before replacing state; malformed input cannot
    // destroy the previous document. Confirm only after the asynchronous read.
    if (!discard()) return;
    editor.open(xml);
    filename = file.name.replace(/\.(xml|fuse)$/i, '') + '.fuse';
    status(`${file.name} · local file`);
  } finally { busy = false; }
}
theme();
try {
  await init(); editor = new WebEditor();
  $('canvas').hidden = false;
  await editor.start($('canvas'));
  $('loading').remove(); theme();
  for (const [id, name] of JSON.parse(editor.products())) {
    $('product').add(new Option(name, id));
  }
  $('product').value = '13';
  document.querySelectorAll('button,select').forEach(el => { el.disabled = false; });
  status('Independent example preset · edits stay in this tab');
  $('open').onclick = () => { $('file').value = ''; $('file').click(); };
  $('file').onchange = () => openFile($('file').files[0]).catch(error);
  $('new').onclick = action(() => { if (discard()) { editor.create(Number($('product').value)); filename = 'Untitled preset.fuse'; status('New preset · download to keep your work'); } });
  $('example').onclick = action(() => { if (discard()) { editor.example(); $('product').value = '13'; filename = 'OpenFUSE example.fuse'; status('Independent example preset'); } });
  $('undo').onclick = action(() => editor.undo());
  $('redo').onclick = action(() => editor.redo());
  $('download').onclick = action(download);
  $('theme').onclick = () => { dark = !dark; theme(); };
  window.addEventListener('beforeunload', e => { if (editor.dirty()) { e.preventDefault(); e.returnValue = ''; } });
  window.addEventListener('keydown', e => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); action(download)(); }
  });
  document.addEventListener('dragover', e => e.preventDefault());
  document.addEventListener('drop', e => { e.preventDefault(); openFile(e.dataTransfer.files[0]).catch(error); });
} catch (e) {
  error(`The browser editor could not start: ${e}\nTry a current browser with WebGL enabled, or download the desktop app.`);
  status('Editor unavailable');
}
