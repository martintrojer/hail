import {
  useBubbleUpMutation,
  useContact,
  useContactNoteMutation,
  useLoginMutation,
  useLogoutMutation,
  useMe,
  useScreenerDecisionMutation,
  useScreenerView,
  useSetupState,
} from './query';
import { queryKeys } from './queryKeys';

export function ApiHookSmoke() {
  void useMe();
  void useSetupState();
  void useScreenerView();
  void useContact('person@example.com');
  void useLoginMutation();
  void useLogoutMutation();
  void useScreenerDecisionMutation();
  void useContactNoteMutation();
  void useBubbleUpMutation();
  void queryKeys.contact('person@example.com');

  return null;
}
