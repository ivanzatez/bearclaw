import { Redirect, Route, Router, Switch } from 'wouter'
import { useHashLocation } from 'wouter/use-hash-location'

import { MemoRecorderHost } from '@/components/MemoRecorder'
import { Scratchpad } from '@/components/Scratchpad'
import { StackUi } from '@/components/stack/StackUi'
import { Toaster } from '@/components/ui/sonner'
import { TooltipProvider } from '@/components/ui/tooltip'
import { DormantProvider } from '@/providers/DormantProvider'
import { BearclawProvider } from '@/providers/BearclawProvider'
import { PrefsProvider } from '@/providers/PrefsProvider'
import { PrsProvider } from '@/providers/PrsProvider'
import { SettingsProvider } from '@/providers/SettingsProvider'
import { SpeechProvider } from '@/providers/SpeechProvider'
import { usePreventFileNavigation } from '@/hooks/useAttach'
import { usePushPresence } from '@/hooks/usePush'
import { useIsDesktop } from '@/hooks/useMediaQuery'
import { ThemeProvider } from '@/providers/ThemeProvider'
import { DesktopShell } from '@/screens/DesktopShell'
import { JobsScreen } from '@/screens/JobsScreen'
import { MemosScreen } from '@/screens/MemosScreen'
import { ListScreen } from '@/screens/ListScreen'
import { NotesScreen } from '@/screens/NotesScreen'
import { OperatorScreen } from '@/screens/OperatorScreen'
import { PrsScreen } from '@/screens/PrsScreen'
import { SessionScreen } from '@/screens/SessionScreen'
import { SettingsScreen } from '@/screens/SettingsScreen'
import { UsageScreen } from '@/screens/UsageScreen'

/**
 * Hash routing: `#/` (list), `#/s/<host>/<session_id>` (detail),
 * plus `#/settings` (this viewer's preferences), `#/usage` (the subscription limits), `#/jobs` (scheduled jobs), `#/operator` (talk to the bearclaw by voice), `#/prs` (the GitHub PRs that need you), `#/memos[/<id>]` (voice memos) and `#/notes[/<host>[/<path>]]` (the notes explorer).
 * No server-side SPA fallback needed; the PWA start_url is `/#/`. Desktop and mobile use the
 * same routes, so links work across both.
 */
export default function App() {
  // ≥ lg: master–detail shell; below it the mobile screens, untouched. Crossing the
  // breakpoint swaps trees (the hash route is shared, so the open session stays open).
  const desktop = useIsDesktop()
  // A file dropped outside a drop zone must not navigate the app away.
  usePreventFileNavigation()
  return (
    <ThemeProvider>
      <PrefsProvider>
      <SettingsProvider>
        <PushPresence />
      <SpeechProvider>
        <BearclawProvider>
        <DormantProvider>
        <PrsProvider>
          <TooltipProvider>
            <Router hook={useHashLocation}>
              {desktop ? (
                <DesktopShell />
              ) : (
                <Switch>
                  <Route path="/">
                    <ListScreen />
                  </Route>
                  <Route path="/settings">
                    <SettingsScreen />
                  </Route>
                  <Route path="/usage">
                    <UsageScreen />
                  </Route>
                  <Route path="/jobs">
                    <JobsScreen />
                  </Route>
                  <Route path="/operator">
                    <OperatorScreen />
                  </Route>
                  <Route path={/^\/memos(?:\/.*)?$/}>
                    <MemosScreen />
                  </Route>
                  <Route path={/^\/prs(?:\/.*)?$/}>
                    <PrsScreen />
                  </Route>
                  <Route path={/^\/notes(?:\/.*)?$/}>
                    <NotesScreen />
                  </Route>
                  <Route path="/s/:host/:id">
                    {(p) => <SessionScreen key={`${p.host}/${p.id}`} host={p.host} id={p.id} />}
                  </Route>
                  <Route>
                    <Redirect to="/" replace />
                  </Route>
                </Switch>
              )}
              {/* Session stacks: the Stack sheet + Spawn sibling form, opened from rows, columns, the session screen. */}
              <StackUi />
              {/* ⌘⇧S / header buttons: the floating (desktop) or drawer (mobile) scratchpad. */}
              <Scratchpad />
              {/* Voice memos: recover unsaved recordings, announce saved ones. */}
              <MemoRecorderHost />
            </Router>
            <Toaster position={desktop ? 'bottom-right' : 'top-center'} offset={desktop ? { bottom: 112, right: 16 } : undefined} />
          </TooltipProvider>
        </PrsProvider>
        </DormantProvider>
        </BearclawProvider>
      </SpeechProvider>
      </SettingsProvider>
      </PrefsProvider>
    </ThemeProvider>
  )
}

/** Tells the server while the app is on screen, so this device is not notified about it (hooks/usePush.ts). */
function PushPresence() {
  usePushPresence()
  return null
}
