import { createContext, useContext, type ReactNode } from "react";

const ActiveContext = createContext(true);

/** Retained pages must release portals and focus traps while off screen. */
export function InteractionScope({
  active,
  children,
}: {
  active: boolean;
  children: ReactNode;
}) {
  return (
    <ActiveContext.Provider value={active}>{children}</ActiveContext.Provider>
  );
}

export const useInteractionActive = () => useContext(ActiveContext);
