//! Every text KLEAN puts in front of a model, in one place.
//!
//! The system prompt, the description of each tool, its parameters, and the
//! sentence that introduces the reduced Lean context: all of it lives here, as
//! plain constants, so it can be read and changed without touching the rest of the
//! code. Nothing else in the crate writes prose for a model.

/// The opening message of every session.
pub const SYSTEM: &str = "\
You are KLEAN's assistant, working inside a Lean 4 project. The user gives you the Lean \
source of the declarations that matter for a request — not the whole project — and a \
question about them.

How to work:
- Read the context before answering. Only use names that appear in it or that you have \
brought in with a tool.
- If you need more of the project, call `view_ast` to look around and `expand` to pull \
declarations in by their fully qualified name. Expanding returns the new, complete context.
- Write Lean as it would appear in a file: no `import` lines, and no surrounding markdown \
fence unless you are marking the block as `lean`.
- Keep answers short and correct. Explain with a sentence or two, only output code if asked to.
- A document the user asked for goes in a ```markdown fence, so KLEAN can show it as a file.
- A picture that explains better than words goes through `diagram`.
- A decision that belongs to the user goes through `ask` or `ask_multi` instead of a guess.
";

/// The line that introduces the user's own instructions, when they added any in
/// `Settings → AI → Customize`.
pub const CUSTOM_INTRO: &str = "Additional instructions from the user, which take precedence over the above:";

/// The line that introduces the reduced Lean source in the conversation.
pub const CONTEXT_INTRO: &str = "Context from the KLEAN project:";

/// What `expand` says back when it brought nodes in.
pub const EXPAND_INTRO: &str = "Nodes added to the context. The context is now:";

// ------------------------------------------------------- tool results (text the model reads)

/// A tool name the model invented.
pub fn no_such_tool(name: &str) -> String {
    format!("There is no tool called `{name}`. Use one of the tools you were given.")
}

/// A tool that failed, reported as its result so the model can recover.
pub fn tool_failed(reason: &str) -> String {
    format!("The tool failed: {reason}")
}

/// A tool KLEAN knows but has no implementation for.
pub fn not_implemented(name: &str) -> String {
    format!("`{name}` is not implemented in this version of KLEAN.")
}

/// The result of `diagram` when the picture was drawn. The model only needs to know
/// it worked; the source stays in KLEAN (it is what the board draws).
pub fn diagram_shown() -> String {
    "The diagram is now on screen.".to_string()
}

/// The result of `diagram` when the board could not draw it.
///
/// The error is mermaid's own complaint, which names the line and the token it
/// wanted — that is what lets the model fix the syntax instead of guessing.
pub fn diagram_failed(attempt: u32, left: u32, error: &str) -> String {
    format!(
        "KLEAN could not draw that diagram; this was attempt {attempt}. Mermaid reported:\n\n\
         ```\n{error}\n```\n\n\
         Fix the syntax and call `diagram` again — {left} retr{} left. Quoting an edge label \
         (`A -->|\"text with punctuation (or arrows)\"| B`) is usually what is missing. If the \
         next attempt fails too, carry on without the picture.",
        if left == 1 { "y" } else { "ies" },
    )
}

/// The result of `diagram` once the retry budget is spent.
pub fn diagram_abandoned(retries: u32) -> String {
    format!(
        "No more attempts at a diagram: {retries} retries were already spent on diagrams \
         mermaid could not draw. Answer in words instead."
    )
}

/// The result of `lean_check` when Lean had nothing to say about the term.
pub fn check_silent(expression: &str) -> String {
    format!("Lean accepted `{expression}` and printed nothing.")
}

/// The result of `lean_check` when Lean could not run at all (no build output, a
/// timeout, a missing toolchain).
pub fn check_failed(reason: &str) -> String {
    format!("`lean_check` could not run: {reason}")
}

/// The result of `expand`: what changed, and the context as it now stands.
pub fn expanded(added: usize, missing: &[String], context: &str) -> String {
    let mut text = format!("{added} declaration(s) added");
    if !missing.is_empty() {
        text.push_str(&format!(". Not found: {}", missing.join(", ")));
    }
    text.push_str(&format!("\n\n{EXPAND_INTRO}\n\n```lean\n{context}\n```"));
    text
}

/// The result of an `ask` the user never answered.
pub const NO_ANSWER: &str = "The user did not answer.";

// ------------------------------------------------------------------- tools

/// Descriptions shown to the model for each tool.
pub mod tool {
    pub const VIEW_AST: &str = "\
Show the project's syntax tree as text, so you can find declarations to expand. \
Returns one line per declaration: kind, fully qualified name, file and signature. \
Use `query` to filter by name, namespace or file path.";

    pub const EXPAND: &str = "\
Add declarations to this session's context by their fully qualified name (namespace plus \
name, for example `Demo.foo`). Returns the new context, which includes the added \
declarations and everything they depend on.";

    pub const DIAGRAM: &str = "\
Show the user a mermaid diagram. Use it to explain structure, a proof plan or a shape of \
data. The diagram is drawn next to your answer.";

    pub const LEAN_CHECK: &str = "\
Ask Lean for the type of an expression, with the whole project in scope — every module the \
project has built is imported first. This is how you confirm what a name means or what a term \
is before you write about it. Returns Lean's own `#check` output: one line per message, and any \
error Lean reported.";

    pub const ASK: &str = "\
Ask the user a question and offer answers. The answer the user picks comes back to you as \
the result of this call. Use it when a decision is theirs, not yours.";

    pub const ASK_MULTI: &str = "\
Ask the user a question where more than one answer may apply. All the answers the user \
picks come back to you as the result of this call.";
}

/// Descriptions of the parameters, per tool.
pub mod param {
    pub const QUERY: &str = "Filter: part of a name, a namespace, or a file path. Empty lists the most important declarations.";
    pub const DEPTH: &str = "How deep to indent the listing (default 2).";
    pub const NAMES: &str = "Fully qualified names to add, e.g. [\"Demo.foo\", \"Demo.bar\"].";
    pub const MERMAID: &str = "The mermaid source, e.g. `graph TD; A-->B`.";
    pub const EXPRESSION: &str = "The Lean term to check, e.g. `Demo.foo` or `(fun x : Nat => x + 1)`.";
    pub const QUESTION: &str = "The question to ask the user.";
    pub const OPTIONS: &str = "The answers the user may pick from.";
}
