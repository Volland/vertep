import VertepLean.Config.Tree
namespace Vertep.CompileP
theorem forIn_ok {α β ε : Type} (l : List α) (init : β) (f : α → β → Except ε (ForInStep β))
    (g : α → β → β) (hf : ∀ x s, f x s = .ok (.yield (g x s))) :
    forIn l init f = .ok (l.foldl (fun s x => g x s) init) := by
  induction l generalizing init with
  | nil => rfl
  | cons a l ih => simp only [List.forIn_cons, List.foldl_cons, hf]; exact ih _

abbrev St := Array CState × Array CTrans × Nat

def trStep (ids : List String) (i : Nat) (s : St) (tr : Transition) : St :=
  (s.1.modify i (fun x => { x with transitions := x.transitions ++ [s.2.1.size] }),
   s.2.1.push { source := i, index := some s.2.2, events := tr.events, cond := tr.cond,
                targets := Compile.resolve ids tr.targets,
                internal := tr.type == .internal, body := tr.body, defaultEntry := false },
   s.2.2 + 1)

def stStep (ids : List String) (s : St) (x : Option State × State) : St :=
  let i := s.2.2
  let r := if (x.2.kind == .compound) = true then
      x.2.transitions.foldl (trStep ids i)
        (s.1.modify i (fun y => { y with initial := some s.2.1.size }),
         s.2.1.push { source := i, index := none, events := [], cond := none,
                      targets := Compile.resolve ids (Compile.resolveInitial x.2).1, internal := true,
                      body := (Compile.resolveInitial x.2).2, defaultEntry := true }, 0)
    else x.2.transitions.foldl (trStep ids i) (s.1, s.2.1, 0)
  (r.1, r.2.1, i + 1)

def hStep (s : Array CState × Array CTrans) (h : Nat) : Array CState × Array CTrans :=
  match (s.1.getD h CState.empty).kind with
  | .history _ =>
    if (s.1.getD h CState.empty).transitions.isEmpty = true then
      (s.1.modify h (fun x => { x with transitions := [s.2.size] }),
       s.2.push { source := h, index := none, events := [], cond := none,
                  targets := match (s.1.getD ((s.1.getD h CState.empty).parent.getD 0) CState.empty).initial with
                    | some ti => (s.2.getD ti CTrans.empty).targets
                    | none => (s.1.getD ((s.1.getD h CState.empty).parent.getD 0) CState.empty).children,
                  internal := false, body := [], defaultEntry := false })
    else s
  | _ => s

def rootInit (c : Chart) : CTrans :=
  { source := 0, index := none, events := [], cond := none,
    targets := Compile.resolve c.ids (if c.initial.isEmpty then Compile.firstChild c.states else c.initial),
    internal := true, body := [], defaultEntry := true }

def init0 (c : Chart) : St :=
  ((Compile.states c).toArray.modify 0 (fun r => { r with initial := some 0 }), #[rootInit c], 1)

def loop1 (c : Chart) : St := c.parented.foldl (stStep c.ids) (init0 c)

def loop2 (c : Chart) : Array CState × Array CTrans :=
  (List.range (loop1 c).1.size).foldl hStep ((loop1 c).1, (loop1 c).2.1)

theorem compile_eq (c : Chart) (cc : CChart) (h : compile c = .ok cc) :
    checkWF c = true ∧ cc.states = (loop2 c).1 ∧ cc.trans = (loop2 c).2 := by
  unfold compile at h
  simp only [bind, Except.bind, pure, Except.pure] at h
  split at h
  · simp [throw, throwThe, MonadExceptOf.throw] at h
  · rename_i hwf
    refine ⟨by simpa using hwf, ?_⟩
    rw [forIn_ok (g := fun x s => stStep c.ids s x)] at h
    · dsimp only at h
      rw [forIn_ok (g := fun x s => hStep s x)] at h
      · simp only [Except.ok.injEq] at h
        subst h
        exact ⟨rfl, rfl⟩
      · intro x s
        unfold hStep
        cases hk : (s.1.getD x CState.empty).kind <;> simp only [hk] <;> (try split) <;> rfl
    · intro x s
      unfold stStep
      split
      all_goals rw [forIn_ok (g := fun tr s1 => trStep c.ids s.2.2 s1 tr)]
      all_goals first | rfl | (intro _ _; rfl)
end Vertep.CompileP
