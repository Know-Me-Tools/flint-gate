import { Navigate, NavLink, Route, Routes } from 'react-router-dom';
import Approvals from '@/pages/Approvals';
import Policies from '@/pages/Policies';

export default function App() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <header className="border-b px-6 py-3 flex items-center gap-6">
        <span className="font-bold text-lg">Flint Gate</span>
        <nav className="flex gap-4">
          <NavLink
            to="/policies"
            className={({ isActive }) =>
              isActive ? 'font-medium' : 'text-muted-foreground hover:text-foreground'
            }
          >
            Policies
          </NavLink>
          <NavLink
            to="/approvals"
            className={({ isActive }) =>
              isActive ? 'font-medium' : 'text-muted-foreground hover:text-foreground'
            }
          >
            Approvals
          </NavLink>
        </nav>
      </header>
      <main className="px-6 py-8 max-w-6xl mx-auto">
        <Routes>
          <Route path="/" element={<Navigate to="/policies" replace />} />
          <Route path="/policies" element={<Policies />} />
          <Route path="/approvals" element={<Approvals />} />
        </Routes>
      </main>
    </div>
  );
}
