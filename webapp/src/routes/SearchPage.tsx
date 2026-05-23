import { Link } from '@tanstack/react-router';
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from 'react';
import {
  HailApiError,
  type ContactNoteSearchResult,
  type MailSearchResult,
  type SearchScope,
} from '../api/client';
import { useSearch } from '../api/query';
import { AppShell } from '../layout/AppShell';

const scopeOptions: Array<{ value: SearchScope; label: string }> = [
  { value: 'all', label: 'All' },
  { value: 'mail', label: 'Mail' },
  { value: 'notes', label: 'Notes' },
  { value: 'clips', label: 'Clips' },
];

function errorMessage(error: Error) {
  if (error instanceof HailApiError) {
    if (error.status === 400 || error.status === 422) {
      return 'Search terms must be at least 2 characters.';
    }
    if (error.status === 401) {
      return 'Your session expired. Sign in again to search.';
    }
    return `Search failed with HTTP ${error.status}.`;
  }

  return 'Search failed. Refresh and try again.';
}

function formatDate(value: string | null | undefined) {
  if (!value) {
    return 'No date';
  }

  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    year: date.getFullYear() === new Date().getFullYear() ? undefined : 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

function StateCard({ title, body }: { title: string; body: string }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 bg-slate-900/40 p-8 text-center">
      <p className="text-base font-semibold text-slate-200">{title}</p>
      <p className="mt-2 max-w-sm text-sm text-slate-400">{body}</p>
    </div>
  );
}

function SearchSkeleton() {
  return (
    <div className="space-y-5" aria-label="Loading search results">
      {['Mail', 'Notes'].map((group) => (
        <section key={group} className="space-y-3">
          <div className="h-4 w-24 animate-pulse rounded bg-slate-800" />
          {Array.from({ length: 2 }, (_, index) => (
            <div
              key={index}
              className="animate-pulse rounded-2xl border border-slate-800 bg-slate-900/60 p-4"
            >
              <div className="h-4 w-2/3 rounded bg-slate-800" />
              <div className="mt-3 h-3 w-full rounded bg-slate-800" />
              <div className="mt-2 h-3 w-1/2 rounded bg-slate-800" />
            </div>
          ))}
        </section>
      ))}
    </div>
  );
}

function MailResultCard({ item }: { item: MailSearchResult }) {
  return (
    <Link
      to="/thread/$threadId"
      params={{ threadId: item.thread_id }}
      className="group block rounded-2xl border border-slate-800 bg-slate-900/70 p-4 transition hover:border-sky-500/60 hover:bg-slate-900 focus:outline-none focus:ring-2 focus:ring-sky-400"
      aria-label={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold text-slate-100">
            {item.from || 'Unknown sender'}
          </p>
          <p className="mt-1 truncate text-base font-semibold text-slate-200">
            {item.subject || '(no subject)'}
          </p>
        </div>
        <time className="shrink-0 text-xs text-slate-500">
          {formatDate(item.received_at)}
        </time>
      </div>
      <p className="mt-2 line-clamp-2 text-sm leading-6 text-slate-400">
        {item.preview || 'No preview available.'}
      </p>
    </Link>
  );
}

function ContactNoteResultCard({ item }: { item: ContactNoteSearchResult }) {
  return (
    <article className="rounded-2xl border border-slate-800 bg-slate-900/70 p-4 shadow-sm shadow-slate-950/40">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold text-emerald-100">
            {item.address || 'Unknown contact'}
          </p>
          <p className="mt-1 text-xs font-semibold uppercase tracking-wide text-slate-500">
            Contact note
          </p>
        </div>
        <time className="shrink-0 text-xs text-slate-500">
          {formatDate(item.updated_at)}
        </time>
      </div>
      <p className="mt-3 line-clamp-4 whitespace-pre-wrap text-sm leading-6 text-slate-300">
        {item.markdown || 'Empty note.'}
      </p>
    </article>
  );
}

function ResultGroup({
  title,
  count,
  children,
}: {
  title: string;
  count: number;
  children: ReactNode;
}) {
  if (count === 0) {
    return null;
  }

  return (
    <section className="space-y-3">
      <h2 className="flex items-center justify-between text-sm font-semibold uppercase tracking-[0.2em] text-slate-400">
        <span>{title}</span>
        <span className="rounded-full bg-slate-900 px-2 py-0.5 text-xs tracking-normal text-slate-500">
          {count}
        </span>
      </h2>
      {children}
    </section>
  );
}

function SearchReading({ submittedQuery }: { submittedQuery: string }) {
  if (submittedQuery.trim().length < 2) {
    return (
      <StateCard
        title="Search mail and notes"
        body="Enter at least 2 characters to search message text and contact notes."
      />
    );
  }

  return (
    <div className="rounded-3xl border border-slate-800 bg-slate-900/60 p-6 shadow-xl shadow-slate-950/30">
      <p className="text-xs font-semibold uppercase tracking-[0.3em] text-sky-300">
        Search
      </p>
      <h2 className="mt-3 text-3xl font-semibold tracking-tight text-slate-50">
        {submittedQuery}
      </h2>
      <p className="mt-3 text-sm leading-6 text-slate-400">
        Results are grouped by source. Mail opens the matching thread; notes show
        the matched contact note inline.
      </p>
    </div>
  );
}

export function SearchPage() {
  const inputId = useId();
  const scopeId = useId();
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [draftQuery, setDraftQuery] = useState('');
  const [submittedQuery, setSubmittedQuery] = useState('');
  const [scope, setScope] = useState<SearchScope>('all');

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      setSubmittedQuery(draftQuery.trim());
    }, 350);

    return () => window.clearTimeout(timeout);
  }, [draftQuery]);

  const query = useSearch({ q: submittedQuery, scope });
  const grouped = useMemo(() => {
    const results = query.data?.results ?? [];
    return {
      mail: results.filter((item) => item.type === 'mail'),
      notes: results.filter((item) => item.type === 'contact_note'),
    };
  }, [query.data?.results]);

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmittedQuery(draftQuery.trim());
  }

  const hasSearch = submittedQuery.length >= 2;
  const resultCount = grouped.mail.length + grouped.notes.length;

  let list;
  if (!hasSearch) {
    list = (
      <StateCard
        title="Ready when you are"
        body="Search requires at least 2 characters. Try a sender, subject, receipt ID, or something from a contact note."
      />
    );
  } else if (query.isPending) {
    list = <SearchSkeleton />;
  } else if (query.isError) {
    list = <StateCard title="Could not search" body={errorMessage(query.error)} />;
  } else if (resultCount === 0) {
    list = (
      <StateCard
        title="No results"
        body={`Nothing matched “${submittedQuery}” in ${scope}.`}
      />
    );
  } else {
    list = (
      <div className="space-y-6">
        <ResultGroup title="Mail" count={grouped.mail.length}>
          <div className="space-y-3">
            {grouped.mail.map((item) => (
              <MailResultCard
                key={`${item.thread_id}:${item.email_id}`}
                item={item}
              />
            ))}
          </div>
        </ResultGroup>
        <ResultGroup title="Contact notes" count={grouped.notes.length}>
          <div className="space-y-3">
            {grouped.notes.map((item) => (
              <ContactNoteResultCard key={item.address} item={item} />
            ))}
          </div>
        </ResultGroup>
      </div>
    );
  }

  const actions = (
    <span className="hidden rounded-full border border-slate-800 px-3 py-1 text-xs font-semibold text-slate-400 sm:inline-flex">
      Ctrl/Cmd-K or / to focus
    </span>
  );

  const form = (
    <form onSubmit={onSubmit} className="mb-5 space-y-3">
      <label htmlFor={inputId} className="block text-sm font-medium text-slate-200">
        Search
        <input
          ref={searchInputRef}
          id={inputId}
          type="search"
          value={draftQuery}
          onChange={(event) => setDraftQuery(event.target.value)}
          placeholder="Search mail, notes, and clips"
          autoComplete="off"
          data-hail-search-input="true"
          className="mt-2 w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-slate-50 outline-none ring-sky-400 transition focus:border-sky-400 focus:ring-2"
        />
      </label>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto] sm:items-end">
        <label htmlFor={scopeId} className="block text-sm font-medium text-slate-200">
          Scope
          <select
            id={scopeId}
            value={scope}
            onChange={(event) => setScope(event.target.value as SearchScope)}
            className="mt-2 w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-slate-100 outline-none ring-sky-400 transition focus:border-sky-400 focus:ring-2"
          >
            {scopeOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <button
          type="submit"
          className="rounded-lg bg-sky-400 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-sky-300"
        >
          Search
        </button>
      </div>
    </form>
  );

  return (
    <AppShell
      title="Search"
      description="Find mail and contact notes across hail."
      actions={actions}
      list={
        <>
          {form}
          {list}
        </>
      }
      reading={<SearchReading submittedQuery={submittedQuery} />}
    />
  );
}
