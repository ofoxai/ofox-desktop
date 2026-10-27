import { useState } from "react";
import { useOfoxAccount } from "@/hooks/useOfoxAccount";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { AssistantLoginRequired, AssistantPage } from "./AssistantPage";

/** Admission comes from MainApp's verified auth state, never the profile cache.
 * Backend IPC still derives the authenticated owner independently. */
export function AccountAssistantPanel(props: {
  enabled: boolean;
  onLoginRequested?: () => void;
}) {
  if (!props.enabled)
    return <AssistantLoginRequired onLoginRequested={props.onLoginRequested} />;
  return <SignedInAssistantPanel onLoginRequested={props.onLoginRequested} />;
}

/** Mounted only within one verified sign-in lifetime. Unknown profile fields
 * can arrive later without replacing the workspace the user already opened. */
function SignedInAssistantPanel({
  onLoginRequested,
}: {
  onLoginRequested?: () => void;
}) {
  const { user } = useOfoxAccount();
  const { apex } = useOfoxApex();
  const accountId = user?.id || null;
  const email = user?.email || null;
  const organization = user?.balance?.organization_id || user?.org_id || null;
  const [boundary, setBoundary] = useState({
    apex,
    accountId,
    email,
    organization,
    generation: 0,
  });
  const replaced =
    apex !== boundary.apex ||
    (accountId !== null &&
      boundary.accountId !== null &&
      accountId !== boundary.accountId) ||
    (email !== null && boundary.email !== null && email !== boundary.email) ||
    // A newly learned stable field needs a matching known field to link it
    // to an already identified profile. Display names are never identity.
    (accountId !== null &&
      boundary.accountId === null &&
      boundary.email !== null &&
      email !== boundary.email) ||
    (email !== null &&
      boundary.email === null &&
      boundary.accountId !== null &&
      accountId !== boundary.accountId) ||
    (organization !== null &&
      boundary.organization !== null &&
      organization !== boundary.organization);
  const nextAccountId = replaced
    ? accountId
    : (accountId ?? boundary.accountId);
  const nextEmail = replaced ? email : (email ?? boundary.email);
  const nextOrganization = replaced
    ? organization
    : (organization ?? boundary.organization);
  if (
    apex !== boundary.apex ||
    nextAccountId !== boundary.accountId ||
    nextEmail !== boundary.email ||
    nextOrganization !== boundary.organization
  ) {
    // Adjust before React commits children: real owner/region changes clear
    // private state immediately. Retain evidence through temporary nulls.
    setBoundary({
      apex,
      accountId: nextAccountId,
      email: nextEmail,
      organization: nextOrganization,
      generation: boundary.generation + Number(replaced),
    });
  }
  return (
    <AssistantPage
      key={boundary.generation}
      onLoginRequested={onLoginRequested}
    />
  );
}
