import { useState, useEffect, useCallback } from "react";
import type { Agent } from "./types";
import { setSessionId, getAgents, listSessions } from "./api";
import SessionPicker from "./components/SessionPicker";
import Dashboard from "./components/Dashboard";

interface Route {
  kind: "sessions" | "dashboard";
  sessionId?: string;
}

function parseHash(): Route {
  const hash = window.location.hash.replace(/^#\/?/, "");
  const match = hash.match(/^sessions\/([^/]+)\/agents/);
  if (match) {
    return { kind: "dashboard", sessionId: match[1] };
  }
  return { kind: "sessions" };
}

function navigate(hash: string): void {
  window.location.hash = hash;
}

export default function App() {
  const [route, setRoute] = useState<Route>(parseHash);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const onHashChange = () => setRoute(parseHash());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  // Validate dashboard session_id against the session list
  useEffect(() => {
    if (route.kind !== "dashboard" || !route.sessionId) {
      setLoading(false);
      return;
    }

    let cancelled = false;

    (async () => {
      try {
        const sessions = await listSessions();
        if (cancelled) return;

        const found = sessions.some((s) => s.session_id === route.sessionId);
        if (!found) {
          navigate("/sessions");
          return;
        }

        setSessionId(route.sessionId!);
        const data = await getAgents();
        if (cancelled) return;
        setAgents(data.agents);
      } catch {
        if (!cancelled) {
          navigate("/sessions");
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [route]);

  const handleSelectSession = useCallback(async (sid: string) => {
    setSessionId(sid);
    try {
      const data = await getAgents();
      setAgents(data.agents);
      navigate(`/sessions/${sid}/agents`);
    } catch {
      setSessionId(null);
    }
  }, []);

  const handleBack = useCallback(() => {
    setSessionId(null);
    setAgents([]);
    navigate("/sessions");
  }, []);

  if (loading && route.kind === "dashboard") {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50">
        <p className="text-gray-400">Loading...</p>
      </div>
    );
  }

  if (route.kind === "dashboard" && route.sessionId) {
    return (
      <Dashboard
        sessionId={route.sessionId}
        initialAgents={agents}
        onBack={handleBack}
      />
    );
  }

  return <SessionPicker onSelect={handleSelectSession} />;
}
