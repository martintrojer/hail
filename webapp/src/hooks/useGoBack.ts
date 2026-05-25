import { useNavigate } from '@tanstack/react-router';

export function useGoBack(fallback = '/imbox') {
  const navigate = useNavigate();

  return () => {
    if (window.history.length > 1) {
      window.history.back();
      return;
    }

    void navigate({ to: fallback });
  };
}
