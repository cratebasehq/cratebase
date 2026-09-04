import { useAuth } from "./hooks/useAuth";
import { AuthScreen } from "./components/AuthScreen";
import { Board } from "./components/Board";

export function App() {
  const { user, register, login, logout } = useAuth();

  if (!user) {
    return <AuthScreen onLogin={login} onRegister={register} />;
  }

  return <Board user={user} onLogout={logout} />;
}
