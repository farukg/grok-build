use crate::app::agent::AgentId;
use crate::app::agent_view::{AgentRole, AgentView, ChildLink};
use indexmap::IndexMap;

#[derive(Default)]
pub struct SessionViews {
    views: IndexMap<AgentId, AgentView>,
}

impl SessionViews {
    pub(crate) fn new() -> Self {
        Self {
            views: IndexMap::new(),
        }
    }

    pub(crate) fn get(&self, id: &AgentId) -> Option<&AgentView> {
        self.views.get(id)
    }
    pub(crate) fn get_mut(&mut self, id: &AgentId) -> Option<&mut AgentView> {
        self.views.get_mut(id)
    }
    pub(crate) fn insert(&mut self, id: AgentId, view: AgentView) -> Option<AgentView> {
        self.views.insert(id, view)
    }
    pub(crate) fn contains_key(&self, id: &AgentId) -> bool {
        self.views.contains_key(id)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
    pub(crate) fn len(&self) -> usize {
        self.views.len()
    }

    pub(crate) fn roots_mut(&mut self) -> impl Iterator<Item = (AgentId, &mut AgentView)> {
        self.views
            .iter_mut()
            .filter_map(|(id, view)| match view.role {
                AgentRole::Root => Some((*id, view)),
                AgentRole::Child(_) => None,
            })
    }

    pub(crate) fn roots(&self) -> impl Iterator<Item = (AgentId, &AgentView)> {
        self.views.iter().filter_map(|(id, view)| match view.role {
            AgentRole::Root => Some((*id, view)),
            AgentRole::Child(_) => None,
        })
    }

    pub(crate) fn children_of(&self, parent: AgentId) -> Vec<AgentId> {
        let mut children: Vec<_> = self
            .views
            .iter()
            .filter_map(|(id, view)| match &view.role {
                AgentRole::Root => None,
                AgentRole::Child(link) if link.parent == parent => {
                    Some((*id, link.started_at, link.subagent_id.as_str()))
                }
                AgentRole::Child(_) => None,
            })
            .collect();
        children.sort_by_key(|(_, started_at, sid)| child_order_key(*started_at, sid));
        children.into_iter().map(|(id, _, _)| id).collect()
    }

    /// Removes `id` together with every session below it: a child view never outlives its parent's.
    pub(crate) fn remove_tree(&mut self, id: AgentId) -> Vec<AgentView> {
        let mut removed = Vec::new();
        let mut pending = vec![id];
        while let Some(next) = pending.pop() {
            pending.extend(self.children_of(next));
            removed.extend(self.views.shift_remove(&next));
        }
        removed
    }

    pub(crate) fn root_of(&self, id: AgentId) -> AgentId {
        let Some(view) = self.views.get(&id) else {
            return id;
        };
        match &view.role {
            AgentRole::Root => id,
            AgentRole::Child(link) => self.root_of(link.parent),
        }
    }

    /// The one exact session-id lookup: roots and children alike.
    pub(crate) fn find_by_session_id(&self, session_id: &str) -> Option<AgentId> {
        self.views.iter().find_map(|(id, view)| {
            view.session
                .session_id
                .as_ref()
                .is_some_and(|sid| &*sid.0 == session_id)
                .then_some(*id)
        })
    }

    pub(crate) fn parent_of(&self, id: AgentId) -> Option<AgentId> {
        match &self.views.get(&id)?.role {
            AgentRole::Root => None,
            AgentRole::Child(link) => Some(link.parent),
        }
    }

    /// A child's link beside its parent's view, which owns the child's subagent row.
    pub(crate) fn link_and_parent(&self, child: AgentId) -> Option<(&ChildLink, &AgentView)> {
        let link = match &self.views.get(&child)?.role {
            AgentRole::Root => return None,
            AgentRole::Child(link) => link,
        };
        Some((link, self.views.get(&link.parent)?))
    }

    pub(crate) fn link_and_parent_mut(
        &mut self,
        child: AgentId,
    ) -> Option<(&ChildLink, &mut AgentView)> {
        let parent = self.parent_of(child).filter(|parent| *parent != child)?;
        let [child_view, parent_view] = self.views.get_disjoint_mut([&child, &parent]);
        match &child_view?.role {
            AgentRole::Root => None,
            AgentRole::Child(link) => Some((link, parent_view?)),
        }
    }

    pub(crate) fn all(&self) -> impl Iterator<Item = (AgentId, &AgentView)> {
        self.views.iter().map(|(id, view)| (*id, view))
    }

    pub(crate) fn all_mut(&mut self) -> impl Iterator<Item = (AgentId, &mut AgentView)> {
        self.views.iter_mut().map(|(id, view)| (*id, view))
    }
}

fn child_order_key(started_at: std::time::Instant, sid: &str) -> (std::time::Instant, &str) {
    (started_at, sid)
}

#[cfg(test)]
impl FromIterator<(AgentId, AgentView)> for SessionViews {
    fn from_iter<I: IntoIterator<Item = (AgentId, AgentView)>>(views: I) -> Self {
        Self {
            views: views.into_iter().collect(),
        }
    }
}

#[cfg(test)]
impl<const N: usize> From<[(AgentId, AgentView); N]> for SessionViews {
    fn from(views: [(AgentId, AgentView); N]) -> Self {
        Self {
            views: IndexMap::from(views),
        }
    }
}

/// The one way tests attach a child session: a normal top-level view whose role links it to `parent`.
#[cfg(test)]
pub(crate) mod test_support {
    use super::SessionViews;
    use crate::app::agent::AgentId;
    use crate::app::agent_view::{AgentRole, AgentView, ChildLink};
    use std::time::Instant;

    pub(crate) fn link_child(
        views: &mut SessionViews,
        parent: AgentId,
        child: AgentId,
        mut view: AgentView,
        started_at: Instant,
    ) {
        let subagent_id = view
            .session
            .session_id
            .as_ref()
            .map(|sid| sid.0.to_string())
            .unwrap_or_else(|| format!("child-{}", child.0));
        let parent_session_id = views
            .get(&parent)
            .and_then(|parent| parent.session.session_id.clone())
            .unwrap_or_else(|| agent_client_protocol::SessionId::new("parent"));
        view.session.id = child;
        view.role = AgentRole::Child(ChildLink {
            parent,
            parent_session_id,
            subagent_id,
            started_at,
        });
        views.insert(child, view);
    }
}

#[cfg(test)]
mod tests {
    use super::SessionViews;
    use super::test_support::link_child;
    use crate::app::agent::AgentId;
    use crate::app::agent_view::AgentView;
    use std::time::{Duration, Instant};

    fn view(sid: &str) -> AgentView {
        crate::app::agent_view::test_agent_view(Some(sid), std::path::PathBuf::from("."))
    }

    #[test]
    fn roots_excludes_children() {
        let root = AgentId(0);
        let mut views = SessionViews::new();
        views.insert(root, view("root"));
        link_child(&mut views, root, AgentId(1), view("child"), Instant::now());
        assert_eq!(views.roots().map(|(id, _)| id).collect::<Vec<_>>(), [root]);
    }

    #[test]
    fn root_of_walks_nested_children() {
        let root = AgentId(0);
        let nested = AgentId(2);
        let mut views = SessionViews::new();
        views.insert(root, view("root"));
        link_child(&mut views, root, AgentId(1), view("child"), Instant::now());
        link_child(
            &mut views,
            AgentId(1),
            nested,
            view("nested"),
            Instant::now(),
        );
        assert_eq!(views.root_of(nested), root);
    }

    #[test]
    fn children_of_is_start_ordered() {
        let root = AgentId(0);
        let start = Instant::now();
        let mut views = SessionViews::new();
        views.insert(root, view("root"));
        link_child(
            &mut views,
            root,
            AgentId(2),
            view("second"),
            start + Duration::from_secs(2),
        );
        link_child(
            &mut views,
            root,
            AgentId(1),
            view("first"),
            start + Duration::from_secs(1),
        );
        assert_eq!(views.children_of(root), [AgentId(1), AgentId(2)]);
    }
}
