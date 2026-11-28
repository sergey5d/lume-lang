import { LumeClient, byteOffsetToUtf16 } from './sdk.js';
import { examples } from './examples.js';
import { createSourceEditor } from './pkg/editor.js';

const client = new LumeClient();
const element = id => document.getElementById(id);
let project;
let activeFile;
let operation;
const editor = createSourceEditor(element('editor'), text => { project.files[activeFile] = text; });

function selectFile(path) {
  activeFile = path;
  editor.open(path, project.files[path]);
  for (const tab of element('files').children) tab.setAttribute('aria-selected', String(tab.dataset.path === path));
}

function renderFiles() {
  element('files').replaceChildren();
  for (const path of Object.keys(project.files)) {
    const button = document.createElement('button');
    button.className = 'file-tab';
    button.role = 'tab';
    button.dataset.path = path;
    button.textContent = path;
    button.addEventListener('click', () => selectFile(path));
    element('files').append(button);
  }
  element('entry-label').textContent = `ENTRY / ${project.entry}`;
  selectFile(activeFile in project.files ? activeFile : project.entry);
}

function setStatus(text, state = 'idle') {
  element('status').textContent = text;
  element('status').dataset.state = state;
}

function loadExample(key) {
  operation?.abort();
  const { entry, files } = examples[key];
  editor.reset();
  project = { entry, files: { ...files } };
  activeFile = entry;
  renderFiles();
  element('diagnostics').replaceChildren();
  element('output').textContent = "Your program's output will appear here.";
  element('return-value').hidden = true;
  setStatus('Ready when you are');
}

for (const [key, example] of Object.entries(examples)) {
  const option = document.createElement('option');
  option.value = key;
  option.textContent = example.label;
  element('examples').append(option);
}
element('examples').addEventListener('change', event => loadExample(event.target.value));

element('add-file').addEventListener('click', () => {
  const path = prompt('Project file path, for example helpers.lum');
  if (!path) return;
  if (Object.hasOwn(project.files, path)) return selectFile(path);
  if (!/^(?:[\w-]+\/)*[\w-]+\.lum$/.test(path)) return setStatus('Use a relative .lum file path', 'error');
  project.files[path] = '';
  activeFile = path;
  renderFiles();
});

function renderDiagnostics(diagnostics, warnings) {
  element('diagnostics').replaceChildren();
  for (const diagnostic of [...diagnostics, ...warnings]) {
    const button = document.createElement('button');
    button.className = `diagnostic ${diagnostic.severity}`;
    const location = document.createElement('span');
    location.textContent = `${diagnostic.path || 'request'}:${diagnostic.span.line}:${diagnostic.span.column} / ${diagnostic.code}`;
    button.append(location, document.createTextNode(diagnostic.message));
    if (diagnostic.helps.length) button.append(document.createElement('br'), document.createTextNode(diagnostic.helps.join(' ')));
    button.addEventListener('click', () => {
      if (!Object.hasOwn(project.files, diagnostic.path)) return;
      selectFile(diagnostic.path);
      editor.select(byteOffsetToUtf16(editor.text, diagnostic.span.start), byteOffsetToUtf16(editor.text, diagnostic.span.end));
    });
    element('diagnostics').append(button);
  }
}

async function perform(action) {
  if (operation) return;
  const controller = new AbortController();
  operation = controller;
  const start = performance.now();
  const sourcePath = activeFile;
  const snapshot = structuredClone(project);
  for (const id of ['check', 'format', 'run']) element(id).disabled = true;
  element('examples').disabled = true;
  element('stop').disabled = false;
  element('diagnostics').replaceChildren();
  element('return-value').hidden = true;
  if (action === 'run') element('output').textContent = '';
  setStatus(action === 'run' ? 'Running locally...' : action === 'check' ? 'Checking...' : 'Formatting...');
  try {
    const options = { signal: controller.signal, onOutput: text => { if (operation === controller) element('output').textContent += text; } };
    const result = action === 'format'
      ? await client.format(snapshot.files[sourcePath], { ...options, path: sourcePath })
      : await client[action](snapshot, options);
    if (operation !== controller) return;
    if (action !== 'format' && JSON.stringify(project) !== JSON.stringify(snapshot)) {
      setStatus('Source changed; check or run again', 'idle');
      return;
    }
    if (action === 'format' && result.ok) {
      if (project.files[sourcePath] !== snapshot.files[sourcePath]) {
        setStatus('Source changed; format was not applied', 'idle');
        return;
      }
      project.files[sourcePath] = result.text;
      editor.replace(sourcePath, result.text);
    }
    if (action === 'run') {
      element('output').textContent = result.output || (result.ok ? 'Program completed without output.' : '');
      if (result.returnValue !== null) {
        element('return-value').textContent = `Returned / ${result.returnValue}`;
        element('return-value').hidden = false;
      }
    }
    renderDiagnostics(result.diagnostics, result.warnings);
    setStatus(result.ok ? action === 'run' ? 'Finished' : action === 'check' ? 'No errors' : 'Formatted' : `${result.diagnostics.length} error${result.diagnostics.length === 1 ? '' : 's'}`, result.ok ? 'success' : 'error');
    element('timing').textContent = `${Math.round(performance.now() - start)} MS / ${result.warnings.length} WARNINGS`;
  } catch (error) {
    if (operation !== controller) return;
    setStatus(error.code === 'cancelled' ? 'Stopped' : error.code === 'timeout' ? 'Timed out' : 'Session failed', 'error');
    if (error.code !== 'cancelled') element('output').textContent += `\n${error.message}\n`;
  } finally {
    if (operation === controller) {
      operation = null;
      for (const id of ['check', 'format', 'run']) element(id).disabled = false;
      element('examples').disabled = false;
      element('stop').disabled = true;
    }
  }
}

for (const action of ['check', 'format', 'run']) element(action).addEventListener('click', () => perform(action));
element('stop').addEventListener('click', () => operation?.abort());
document.addEventListener('keydown', event => {
  if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); perform('run'); }
});
window.addEventListener('pagehide', () => client.dispose());
loadExample('hello');
for (const id of ['examples', 'add-file', 'check', 'format', 'run']) element(id).disabled = false;
