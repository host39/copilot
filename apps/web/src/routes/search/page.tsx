import type { SearchHitResponse } from "@copilot/contracts";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { describeError } from "@web/api/errors";
import { keys } from "@web/api/routes/keys";
import { search } from "@web/api/routes/search";
import { FormAlert } from "@web/components/Form/FormAlert";
import { PageTitle } from "@web/components/Layout/PageTitle";
import { PageLoading } from "@web/components/Suspense/PageLoading";
import {
  highlight,
  hitLink,
  queryProblem,
  SEARCH_MAX_CHARS,
  terms,
} from "@web/routes/search/highlight";
import { useEffect, useState } from "react";
import { LuMessageSquare, LuSearch } from "react-icons/lu";
import { Link, useSearchParams } from "react-router-dom";

/** The query parameter that holds the search text, so a search can be shared and reloaded. */
const QUERY_PARAM = "q";
/** Results per search (the server's cap). */
const RESULTS = 50;
/** Typing pauses this long before the search runs. */
const DEBOUNCE_MS = 300;

/** `text` with the parts that match `words` marked. */
function Marked({ text, words }: { text: string; words: string[] }) {
  return (
    <>
      {highlight(text, words).map((part, index) =>
        part.match ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: parts never move
          <mark key={index} className="rounded bg-accent/20 text-fg">
            {part.text}
          </mark>
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: parts never move
          <span key={index}>{part.text}</span>
        )
      )}
    </>
  );
}

function Hit({ hit, words }: { hit: SearchHitResponse; words: string[] }) {
  const title = hit.chatTitle.trim() || "New chat";
  return (
    <li>
      <Link to={hitLink(hit)} className="list-row items-start">
        <LuMessageSquare aria-hidden className="mt-0.5 shrink-0 text-muted" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium">
            {hit.messageId === null ? <Marked text={title} words={words} /> : title}
          </p>
          <p className="mt-0.5 line-clamp-2 text-sm break-words text-muted">
            {hit.messageId === null ? (
              "Matched the title"
            ) : (
              <Marked text={hit.snippet} words={words} />
            )}
          </p>
        </div>
      </Link>
    </li>
  );
}

/** Search: chat titles and message text, with the matching words marked. */
export default function SearchPage() {
  const [params, setParams] = useSearchParams();
  const query = params.get(QUERY_PARAM) ?? "";
  const [text, setText] = useState(query);
  const problem = queryProblem(query);
  const results = useQuery({
    queryKey: keys.search(query.trim()),
    queryFn: ({ signal }) => search(query.trim(), RESULTS, signal),
    enabled: problem === null,
    placeholderData: keepPreviousData,
  });

  // Typing updates the address after a pause, which runs the search.
  useEffect(() => {
    if (text === query) return;
    const timer = setTimeout(() => {
      setParams(text.trim() ? { [QUERY_PARAM]: text } : {}, { replace: true });
    }, DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [text, query, setParams]);

  const words = terms(query);
  const list = results.data ?? [];
  const tooLong = query.trim() !== "" && problem !== null;

  return (
    <section className="screen max-w-3xl">
      <PageTitle page="Search" />
      <h1 className="text-xl font-semibold tracking-tight">Search</h1>
      <search className="mt-4 block">
        <form
          className="relative"
          onSubmit={(event) => {
            event.preventDefault();
            setParams(text.trim() ? { [QUERY_PARAM]: text } : {}, { replace: true });
          }}
        >
          <label htmlFor="search" className="sr-only">
            Search chats
          </label>
          <LuSearch
            aria-hidden
            className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-muted"
          />
          <input
            id="search"
            type="search"
            value={text}
            onChange={(event) => setText(event.target.value)}
            placeholder="Search chats"
            maxLength={SEARCH_MAX_CHARS * 2}
            autoComplete="off"
            // biome-ignore lint/a11y/noAutofocus: the screen exists to type a search
            autoFocus
            className="field-input mt-0 pl-9"
          />
        </form>
      </search>

      <div className="mt-6 space-y-3" aria-live="polite" aria-busy={results.isFetching}>
        {tooLong ? <FormAlert>{problem}</FormAlert> : null}
        {problem === null && results.isError ? (
          <FormAlert>{describeError(results.error).message}</FormAlert>
        ) : null}
        {problem === null && results.isPending ? <PageLoading /> : null}
        {problem === null && results.isSuccess && list.length === 0 ? (
          <p className="text-sm text-muted">{`No chats match “${query.trim()}”.`}</p>
        ) : null}
        {query.trim() === "" ? (
          <p className="text-sm text-muted">Find chats by their title or any message in them.</p>
        ) : null}
        {problem === null && list.length > 0 ? (
          <ul aria-label="Results" className="list-panel">
            {list.map((hit) => (
              <Hit key={`${hit.chatId}-${hit.messageId ?? "title"}`} hit={hit} words={words} />
            ))}
          </ul>
        ) : null}
      </div>
    </section>
  );
}
