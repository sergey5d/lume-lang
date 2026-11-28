import { EditorState } from '@codemirror/state';
import { EditorView, drawSelection, highlightActiveLineGutter, highlightSpecialChars, keymap, lineNumbers } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentWithTab, isolateHistory } from '@codemirror/commands';
import { HighlightStyle, StreamLanguage, bracketMatching, indentUnit, syntaxHighlighting } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { lumeParser } from './lume-language.js';

// The editor build injects the current keyword table from the Rust lexer.
const language = StreamLanguage.define(lumeParser(LUME_KEYWORDS));
const highlighting = HighlightStyle.define([
  { tag: tags.keyword, class: 'lume-keyword' },
  { tag: tags.bool, class: 'lume-bool' },
  { tag: tags.string, class: 'lume-string' },
  { tag: tags.comment, class: 'lume-comment' },
  { tag: tags.number, class: 'lume-number' },
  { tag: tags.typeName, class: 'lume-type' },
]);

export function createSourceEditor(parent, onChange) {
  const saved = new Map();
  let activePath;
  const extensions = [
    lineNumbers(),
    highlightActiveLineGutter(),
    highlightSpecialChars(),
    drawSelection(),
    history(),
    bracketMatching(),
    language,
    syntaxHighlighting(highlighting),
    indentUnit.of('    '),
    EditorState.tabSize.of(4),
    // Ctrl/Cmd+Enter belongs to the playground's Run action, not insertBlankLine.
    keymap.of([indentWithTab, ...defaultKeymap.filter(binding => binding.key !== 'Mod-Enter'), ...historyKeymap]),
    EditorView.contentAttributes.of({
      'aria-label': 'Source editor',
      'aria-multiline': 'true',
      spellcheck: 'false',
      autocorrect: 'off',
      autocapitalize: 'off',
    }),
    EditorView.updateListener.of(update => {
      if (update.docChanged) onChange(update.state.doc.toString());
    }),
  ];
  parent.replaceChildren();
  const view = new EditorView({ parent, extensions });
  function replacement(state, text) {
    const { anchor, head } = state.selection.main;
    return {
      changes: { from: 0, to: state.doc.length, insert: text },
      selection: { anchor: Math.min(anchor, text.length), head: Math.min(head, text.length) },
      annotations: isolateHistory.of('full'),
    };
  }
  return {
    get text() { return view.state.doc.toString(); },
    reset() { saved.clear(); activePath = undefined; },
    open(path, text) {
      if (activePath === path && view.state.doc.toString() === text) return;
      if (activePath) saved.set(activePath, view.state);
      activePath = path;
      const state = saved.get(path);
      view.setState(state?.doc.toString() === text ? state : EditorState.create({ doc: text, extensions }));
    },
    replace(path, text) {
      const state = activePath === path ? view.state : saved.get(path);
      if (!state || state.doc.toString() === text) return;
      if (activePath === path) view.dispatch(replacement(state, text));
      else saved.set(path, state.update(replacement(state, text)).state);
    },
    select(from, to) {
      const end = view.state.doc.length;
      view.dispatch({
        selection: { anchor: Math.min(from, end), head: Math.min(to, end) },
        effects: EditorView.scrollIntoView(from, { y: 'center' }),
      });
      view.focus();
    },
  };
}
