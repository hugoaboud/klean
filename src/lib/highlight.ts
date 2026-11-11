// Shared highlight.js instance with the Lean grammar registered once.

import hljs from "highlight.js/lib/core";
import lean from "./lean-language";

hljs.registerLanguage("lean", lean);

export default hljs;
