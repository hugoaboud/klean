// Markdown for model output, with the diagram blocks pulled out.
//
// `marked` renders the markdown (it is the library the ecosystem uses) and
// `dompurify` sanitises the result, because the text comes from a model and ends
// up as HTML in the webview. Fenced ```mermaid blocks are cut out first: they are
// not markdown, they are diagrams, and they get their own component.

import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/core";
import { marked } from "marked";
import lean from "./lean-language";

hljs.registerLanguage("lean", lean);

export { splitBlocks, hasDiagram } from "./blocks";
export type { Block } from "./blocks";

const renderer = new marked.Renderer();

// Code is highlighted with the editor's own grammar, so a snippet in an answer
// looks like a snippet in the editor.
renderer.code = ({ text, lang }) => {
  const language = (lang ?? "").trim().toLowerCase();
  let html: string;
  try {
    html = language
      ? hljs.highlight(text, { language, ignoreIllegals: true }).value
      : hljs.highlightAuto(text).value;
  } catch {
    html = escapeHtml(text);
  }
  return `<pre class="md-code hljs"><code>${html}</code></pre>`;
};

marked.setOptions({ gfm: true, breaks: true, renderer });

/** Renders markdown to sanitised HTML. */
export function renderMarkdown(text: string): string {
  const html = marked.parse(text, { async: false }) as string;
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    // Links come from a model: let them out, but never as `javascript:`.
    ALLOWED_URI_REGEXP: /^(?:https?|mailto|file):/i,
  });
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
