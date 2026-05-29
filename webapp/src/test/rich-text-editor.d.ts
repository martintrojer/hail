interface Window {
  __HAIL_TEST_EDITOR_UPDATES__?: WeakMap<HTMLElement, (html: string) => void>;
  __HAIL_TEST_EDITORS__?: WeakMap<HTMLElement, import('@tiptap/react').Editor>;
}
