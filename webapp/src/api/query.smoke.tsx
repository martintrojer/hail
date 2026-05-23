import {
  useBubbleUpMutation,
  useContact,
  useContactNoteMutation,
  useFeedView,
  useImboxView,
  useLoginMutation,
  useLogoutMutation,
  useMe,
  usePapertrailView,
  useScreenerDecisionMutation,
  useScreenerView,
  useSetupState,
} from './query';
import { queryKeys } from './queryKeys';

export function ApiHookSmoke() {
  void useMe();
  void useSetupState();
  void useScreenerView();
  void useImboxView();
  void useFeedView();
  void usePapertrailView();
  void useContact('person@example.com');
  void useLoginMutation();
  void useLogoutMutation();
  void useScreenerDecisionMutation();
  void useContactNoteMutation();
  void useBubbleUpMutation();
  void queryKeys.contact('person@example.com');
  void queryKeys.view('imbox');

  return null;
}
