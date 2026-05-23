import {
  createContext,
  useContext,
  useMemo,
  type PropsWithChildren,
} from 'react';
import { useNavigate } from '@tanstack/react-router';
import { defaultApiClient, useLogoutMutation, useMe } from '../api/query';
import type { UserView } from '../api/client';

interface AuthContextValue {
  user: UserView | null;
  loading: boolean;
  logout: () => void;
  logoutLoading: boolean;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: PropsWithChildren) {
  const navigate = useNavigate();
  const meQuery = useMe(defaultApiClient, {
    retry: false,
  });
  const logoutMutation = useLogoutMutation(defaultApiClient, {
    onSuccess: () => {
      void navigate({ to: '/login' });
    },
  });

  const value = useMemo<AuthContextValue>(
    () => ({
      user: meQuery.data?.user ?? null,
      loading: meQuery.isPending,
      logout: () => logoutMutation.mutate(),
      logoutLoading: logoutMutation.isPending,
    }),
    [
      meQuery.data?.user,
      meQuery.isPending,
      logoutMutation,
    ],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth() {
  const value = useContext(AuthContext);
  if (value === null) {
    throw new Error('useAuth must be used inside AuthProvider');
  }
  return value;
}
