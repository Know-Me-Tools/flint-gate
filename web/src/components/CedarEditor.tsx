import { useEffect, useRef } from 'react';
import { EditorView, basicSetup } from 'codemirror';
import { Compartment, EditorState } from '@codemirror/state';
import { HighlightStyle, StreamLanguage, syntaxHighlighting } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { simpleMode } from '@codemirror/legacy-modes/mode/simple-mode';

const CEDAR_KEYWORDS =
  /\b(permit|forbid|when|unless|principal|action|resource|is|in|has|if|then|else|true|false|like|context|ip|decimal)\b/;

const cedarLanguage = StreamLanguage.define(
  simpleMode({
    start: [
      { regex: /\/\/.*/, token: 'comment' },
      { regex: /"(?:[^\\]|\\.)*?"/, token: 'string' },
      { regex: /@[A-Za-z_]\w*/, token: 'meta' },
      { regex: CEDAR_KEYWORDS, token: 'keyword' },
      { regex: /[A-Za-z_][\w:]*/, token: 'variable' },
      { regex: /[{};,()[\]]/, token: 'punctuation' },
    ],
  }),
);

const cedarHighlight = HighlightStyle.define([
  { tag: tags.keyword, color: '#7c3aed', fontWeight: '600' },
  { tag: tags.comment, color: '#6b7280', fontStyle: 'italic' },
  { tag: tags.string, color: '#059669' },
  { tag: tags.meta, color: '#d97706' },
  { tag: tags.variableName, color: '#1d4ed8' },
]);

const editableCompartment = new Compartment();

interface CedarEditorProps {
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
}

export function CedarEditor({ value, onChange, disabled = false }: CedarEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const startState = EditorState.create({
      doc: value,
      extensions: [
        basicSetup,
        cedarLanguage,
        syntaxHighlighting(cedarHighlight),
        editableCompartment.of(EditorView.editable.of(!disabled)),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onChange(update.state.doc.toString());
          }
        }),
        EditorView.theme({
          '&': {
            fontSize: '13px',
            fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
            border: '1px solid hsl(var(--border))',
            borderRadius: 'calc(var(--radius) - 2px)',
            minHeight: '120px',
          },
          '.cm-content': { padding: '8px 12px' },
          '.cm-focused': { outline: 'none' },
        }),
      ],
    });

    const view = new EditorView({ state: startState, parent: containerRef.current });
    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== value) {
      view.dispatch({
        changes: { from: 0, to: current.length, insert: value },
      });
    }
  }, [value]);

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: editableCompartment.reconfigure(EditorView.editable.of(!disabled)),
    });
  }, [disabled]);

  return <div ref={containerRef} className="cedar-editor" />;
}
