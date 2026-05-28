import { useMemo, useState } from 'react';
import { useApiClient } from '../api/ApiClientProvider';
import type { HailApiClient, LabelResponse } from '../api/client';
import { useAssignLabelToThreadsMutation, useLabels } from '../api/query';
import { Plus, Tags } from './icons';
import { Button } from './ui/button';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from './ui/command';
import {
  Popover,
  PopoverContent,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from './ui/popover';

export interface BatchLabelPickerProps {
  client?: HailApiClient;
  count: number;
  threadIds: string[];
  onAssigned: () => void;
}

function normalizeLabelName(name: string) {
  return name
    .split('/')
    .map((segment) => segment.trim().replace(/\s+/g, ' '))
    .join('/')
    .toLocaleLowerCase();
}

function labelLeafText(label: LabelResponse) {
  const leaf = label.leaf_name.trim();
  if (leaf.length > 0) {
    return leaf;
  }
  return label.path_segments.at(-1)?.trim() || label.name;
}

function sortLabels(labels: LabelResponse[]) {
  return [...labels].sort((left, right) =>
    left.name.localeCompare(right.name, undefined, { sensitivity: 'base' }),
  );
}

export function BatchLabelPicker({
  client,
  count,
  threadIds,
  onAssigned,
}: BatchLabelPickerProps) {
  const apiClient = client ?? useApiClient();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const [error, setError] = useState<string | null>(null);
  const labelsQuery = useLabels(apiClient, { enabled: open });
  const assignLabel = useAssignLabelToThreadsMutation(apiClient);

  const allLabels = sortLabels(labelsQuery.data?.labels ?? []);
  const trimmedSearch = search.trim();
  const normalizedSearch = normalizeLabelName(trimmedSearch);
  const exactMatch = useMemo(
    () => allLabels.some((label) => normalizeLabelName(label.name) === normalizedSearch),
    [allLabels, normalizedSearch],
  );
  const canCreate = trimmedSearch.length > 0 && !exactMatch;
  const busy = assignLabel.isPending;

  async function assignExisting(label: LabelResponse) {
    if (busy) {
      return;
    }
    setError(null);
    try {
      await assignLabel.mutateAsync({ thread_ids: threadIds, label_id: label.id });
      setOpen(false);
      setSearch('');
      onAssigned();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Label assignment failed');
    }
  }

  async function createAndAssign() {
    if (!canCreate || busy) {
      return;
    }
    setError(null);
    try {
      await assignLabel.mutateAsync({ thread_ids: threadIds, label_name: trimmedSearch });
      setOpen(false);
      setSearch('');
      onAssigned();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Label assignment failed');
    }
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button type="button" size="sm" variant="outline">
          <Tags data-icon="inline-start" />
          Label
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-2">
        <PopoverHeader>
          <PopoverTitle>Assign label</PopoverTitle>
        </PopoverHeader>
        <p className="px-2 pb-2 text-xs leading-5 text-muted-foreground">
          Add a label to {count} selected thread{count === 1 ? '' : 's'}. Existing labels are kept.
        </p>
        <Command shouldFilter>
          <CommandInput
            value={search}
            onValueChange={setSearch}
            placeholder="Search or create label…"
            disabled={busy}
          />
          <CommandList>
            <CommandEmpty>No labels found.</CommandEmpty>
            <CommandGroup heading="Existing labels">
              {allLabels.map((label) => (
                <CommandItem
                  key={label.id}
                  value={label.name}
                  aria-label={`Assign label ${label.name}`}
                  disabled={busy}
                  onSelect={() => {
                    void assignExisting(label);
                  }}
                >
                  <Tags data-icon="inline-start" />
                  <span className="min-w-0 flex-1 truncate" title={label.name}>
                    {labelLeafText(label)}
                  </span>
                  {label.name !== labelLeafText(label) ? (
                    <CommandShortcut className="max-w-28 truncate normal-case tracking-normal">
                      {label.name}
                    </CommandShortcut>
                  ) : null}
                </CommandItem>
              ))}
            </CommandGroup>
            {canCreate ? (
              <>
                <CommandSeparator />
                <CommandGroup>
                  <CommandItem
                    value={`create-${trimmedSearch}`}
                    disabled={busy}
                    onSelect={() => {
                      void createAndAssign();
                    }}
                  >
                    <Plus data-icon="inline-start" />
                    Create and assign “{trimmedSearch}”
                  </CommandItem>
                </CommandGroup>
              </>
            ) : null}
          </CommandList>
        </Command>
        {error ? (
          <p role="alert" className="px-2 text-xs text-destructive">
            {error}
          </p>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}
