import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from 'react';
import type {
  ContactNoteSearchResult,
  MailSearchResult,
  SearchScope,
} from '../api/client';
import { useSearch } from '../api/query';
import { ListView } from '../components/ListView';
import { StateCard } from '../components/StateCard';
import { ThreadLink } from '../components/ThreadLink';
import { AppShell } from '../layout/AppShell';
import { formatDateTime } from '../lib/dates';
import { viewErrorMessage } from '../lib/errorMessages';

const scopeOptions: Array<{ value: SearchScope; label: string }> = [
  { value: 'all', label: 'All' },
  { value: 'mail', label: 'Mail' },
  { value: 'notes', label: 'Notes' },
];

function SearchSkeleton() {
  return (
    <div className="space-y-5" aria-label="Loading search results">
      {['Mail', 'Notes'].map((group) => (
        <section key={group} className="space-y-3">
          <div className="h-4 w-24 animate-pulse rounded bg-hover" />
          {Array.from({ length: 2 }, (_, index) => (
            <div
              key={index}
              className="animate-pulse rounded-lg border border-hairline bg-surface p-4"
            >
              <div className="h-4 w-2/3 rounded bg-hover" />
              <div className="mt-3 h-3 w-full rounded bg-hover" />
              <div className="mt-2 h-3 w-1/2 rounded bg-hover" />
            </div>
          ))}
        </section>
      ))}
    </div>
  );
}

function MailResultCard({ item }: { item: MailSearchResult }) {
  return (
    <ThreadLink
      threadId={item.thread_id}
      className="group block rounded-lg border border-hairline bg-surface p-4 transition hover:border-accent-blue hover:bg-hover focus:outline-none focus:ring-2 focus:ring-accent-blue"
      ariaLabel={`Open ${item.subject || 'thread'} from ${item.from || 'unknown sender'}`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold text-ink-primary">
            {item.from || 'Unknown sender'}
          </p>
          <p className="mt-1 truncate text-base font-semibold text-ink-primary">
            {item.subject || '(no subject)'}
          </p>
        </div>
        <time className="shrink-0 text-xs text-ink-primary0">
          {formatDateTime(item.received_at)}
        </time>
      </div>
      <p className="mt-2 line-clamp-2 text-sm leading-6 text-ink-secondary">
        {item.preview || 'No preview available.'}
      </p>
    </ThreadLink>
  );
}

function ContactNoteResultCard({ item }: { item: ContactNoteSearchResult }) {
  return (
    <article className="rounded-lg border border-hairline bg-surface p-4 shadow-sm">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold text-accent-blue">
            {item.address || 'Unknown contact'}
          </p>
          <p className="mt-1 text-xs font-semibold uppercase tracking-wide text-ink-primary0">
            Contact note
          </p>
        </div>
        <time className="shrink-0 text-xs text-ink-primary0">
          {formatDateTime(item.updated_at)}
        </time>
      </div>
      <p className="mt-3 line-clamp-4 whitespace-pre-wrap text-sm leading-6 text-ink-secondary">
        {item.markdown || 'Empty note.'}
      </p>
    </article>
  );
}

function ResultGroup<T>({
  title,
  items,
  renderItem,
  keyExtractor,
}: {
  title: string;
  items: T[];
  renderItem: (item: T, index: number) => ReactNode;
  keyExtractor: (item: T) => string;
}) {
  if (items.length === 0) {
    return null;
  }

  return (
    <section className="space-y-3">
      <h2 className="flex items-center justify-between text-sm font-semibold uppercase tracking-[0.2em] text-ink-secondary">
        <span>{title}</span>
        <span className="rounded-full bg-hover px-2 py-0.5 text-xs tracking-normal text-ink-primary0">
          {items.length}
        </span>
      </h2>
      <div className="space-y-3">
        <ListView
          items={items}
          renderItem={renderItem}
          keyExtractor={keyExtractor}
          hasMore={false}
          isLoadingMore={false}
          onLoadMore={() => {}}
          emptyState={null}
        />
      </div>
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
    <div className="rounded-lg border border-hairline bg-surface p-6 shadow-md">
      <p className="text-xs font-semibold uppercase tracking-[0.3em] text-accent-blue">
        Search
      </p>
      <h2 className="mt-3 text-3xl font-semibold tracking-tight text-ink-primary">
        {submittedQuery}
      </h2>
      <p className="mt-3 text-sm leading-6 text-ink-secondary">
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
    list = <StateCard title="Could not search" body={viewErrorMessage(query.error, 'Search')} />;
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
        <ResultGroup
          title="Mail"
          items={grouped.mail}
          renderItem={(item) => <MailResultCard item={item} />}
          keyExtractor={(item) => `${item.thread_id}:${item.email_id}`}
        />
        <ResultGroup
          title="Contact notes"
          items={grouped.notes}
          renderItem={(item) => <ContactNoteResultCard item={item} />}
          keyExtractor={(item) => item.address}
        />
      </div>
    );
  }

  const actions = (
    <span className="hidden rounded-full border border-hairline px-3 py-1 text-xs font-semibold text-ink-secondary sm:inline-flex">
      Ctrl/Cmd-K or / to focus
    </span>
  );

  const form = (
    <form onSubmit={onSubmit} className="mb-5 space-y-3">
      <label htmlFor={inputId} className="block text-sm font-medium text-ink-primary">
        Search
        <input
          ref={searchInputRef}
          id={inputId}
          type="search"
          value={draftQuery}
          onChange={(event) => setDraftQuery(event.target.value)}
          placeholder="Search mail and notes"
          autoComplete="off"
          data-hail-search-input="true"
          className="mt-2 w-full rounded-lg border border-hairline bg-hover px-3 py-2 text-ink-primary outline-none ring-accent-blue transition focus:border-accent-blue focus:ring-2"
        />
      </label>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto] sm:items-end">
        <label htmlFor={scopeId} className="block text-sm font-medium text-ink-primary">
          Scope
          <select
            id={scopeId}
            value={scope}
            onChange={(event) => setScope(event.target.value as SearchScope)}
            className="mt-2 w-full rounded-lg border border-hairline bg-page px-3 py-2 text-ink-primary outline-none ring-accent-blue transition focus:border-accent-blue focus:ring-2"
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
          className="rounded-full bg-accent-blue px-4 py-2 text-sm font-semibold text-white transition hover:bg-accent-blue-hover"
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
