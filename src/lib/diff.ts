// A line-by-line diff, for the side-by-side view of what a model changed.
//
// The feed shows a declaration the way an editor shows a change: the old text on
// the left, the new on the right, aligned so a reader can see which lines moved
// rather than two unrelated blocks. Alignment is a longest-common-subsequence walk
// over the lines, which is exact and fast enough for a declaration.

/** One line of the comparison; `null` where a side has nothing. */
export interface DiffRow {
  left: string | null;
  right: string | null;
  /** `same`, `add` (right only) or `remove` (left only). */
  kind: "same" | "add" | "remove";
}

/** Lines of text, without a trailing empty line invented by the final newline. */
function lines(text: string): string[] {
  const all = text.replace(/\r\n/g, "\n").split("\n");
  if (all.length > 0 && all[all.length - 1] === "") all.pop();
  return all;
}

/** The longest common subsequence of two line arrays, as index pairs. */
function common(before: string[], after: string[]): [number, number][] {
  const rows = before.length;
  const columns = after.length;
  // `lengths[i][j]` is the LCS length of `before[i..]` and `after[j..]`.
  const lengths: number[][] = Array.from({ length: rows + 1 }, () =>
    new Array<number>(columns + 1).fill(0),
  );
  for (let i = rows - 1; i >= 0; i -= 1) {
    for (let j = columns - 1; j >= 0; j -= 1) {
      lengths[i][j] =
        before[i] === after[j]
          ? lengths[i + 1][j + 1] + 1
          : Math.max(lengths[i + 1][j], lengths[i][j + 1]);
    }
  }
  const pairs: [number, number][] = [];
  let i = 0;
  let j = 0;
  while (i < rows && j < columns) {
    if (before[i] === after[j]) {
      pairs.push([i, j]);
      i += 1;
      j += 1;
    } else if (lengths[i + 1][j] >= lengths[i][j + 1]) {
      i += 1;
    } else {
      j += 1;
    }
  }
  return pairs;
}

/**
 * Two texts, aligned for a side-by-side view.
 *
 * Lines that only exist on one side get an empty cell opposite them, so both
 * columns stay the same height and a change reads as one row.
 */
export function sideBySide(before: string, after: string): DiffRow[] {
  const left = lines(before);
  const right = lines(after);
  const rows: DiffRow[] = [];
  let i = 0;
  let j = 0;
  const drop = (until: number) => {
    for (; i < until; i += 1) rows.push({ left: left[i], right: null, kind: "remove" });
  };
  const add = (until: number) => {
    for (; j < until; j += 1) rows.push({ left: null, right: right[j], kind: "add" });
  };
  for (const [matchLeft, matchRight] of common(left, right)) {
    drop(matchLeft);
    add(matchRight);
    rows.push({ left: left[matchLeft], right: right[matchRight], kind: "same" });
    i = matchLeft + 1;
    j = matchRight + 1;
  }
  drop(left.length);
  add(right.length);
  return rows;
}

/** How many lines a change touched, for a summary line. */
export function changedLines(rows: DiffRow[]): { added: number; removed: number } {
  return {
    added: rows.filter((row) => row.kind === "add").length,
    removed: rows.filter((row) => row.kind === "remove").length,
  };
}
