//! Receiver-aware resolution of tree-sitter call sites.
//!
//! A call is matched against project declarations in this order:
//! 1. receiver: `self.`/implicit → members of the enclosing type, its
//!    extensions and supertypes (nearest level wins); `Type.` or a variable
//!    of a declared type → members of that type; `super.` → supertypes only;
//!    implicit calls fall back to free functions and top-level types;
//!    unknown receivers consider every member with that name, except for
//!    well-known standard library / SwiftUI / UIKit / Combine names, which
//!    never resolve to project symbols through an unknown receiver;
//! 2. visibility: `private`/`fileprivate` targets only from the same file;
//! 3. arguments: labels must fit the target's parameter labels; exact
//!    matches (same count) win over matches relying on default values.
//!
//! One remaining candidate → a normal edge. Two up to
//! [`ResolutionConfig::max_candidates`] → edges flagged `ambiguous`, which
//! navigation shows but analytics ignore. More candidates or none → no edge;
//! if the name exists in the project it is recorded in `name_refs`
//! so dead-code treats symbols with that name as possibly used. Names read
//! outside of calls (`Units.second`, `let x: Request`) are recorded there too.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::config::ResolutionConfig;
use crate::tree_sitter::parser::{CallSite, Receiver};

/// Names that, called on a receiver of unknown type, almost always hit the
/// standard library, Foundation, SwiftUI, UIKit or Combine.
const LIBRARY_NAMES: &[&str] = &[
    // Sequence / Collection / String / Optional
    "map",
    "flatMap",
    "compactMap",
    "filter",
    "reduce",
    "forEach",
    "sorted",
    "sort",
    "first",
    "last",
    "contains",
    "allSatisfy",
    "min",
    "max",
    "count",
    "enumerated",
    "reversed",
    "joined",
    "split",
    "append",
    "appending",
    "insert",
    "remove",
    "removeAll",
    "removeFirst",
    "removeLast",
    "removeValue",
    "updateValue",
    "index",
    "firstIndex",
    "lastIndex",
    "prefix",
    "suffix",
    "dropFirst",
    "dropLast",
    "zip",
    "merge",
    "merging",
    "mapValues",
    "compactMapValues",
    "grouped",
    "union",
    "intersection",
    "subtracting",
    "isSubset",
    "starts",
    "hasPrefix",
    "hasSuffix",
    "replacingOccurrences",
    "trimmingCharacters",
    "components",
    "lowercased",
    "uppercased",
    "capitalized",
    "localizedCaseInsensitiveContains",
    "range",
    "addingPercentEncoding",
    "data",
    "encode",
    "decode",
    "description",
    "hash",
    "isEqual",
    "copy",
    "formatted",
    "string",
    "date",
    "addingTimeInterval",
    "timeIntervalSince",
    "distance",
    "advanced",
    "rounded",
    "round",
    "clamped",
    "shuffled",
    "randomElement",
    "partition",
    "swapAt",
    "elementsEqual",
    "lexicographicallyPrecedes",
    "withUnsafeBytes",
    "subdata",
    "base64EncodedString",
    "hexString",
    "write",
    "read",
    "open",
    "close",
    "resume",
    "cancel",
    "suspend",
    "value",
    "get",
    "set",
    "callAsFunction",
    // Combine
    "sink",
    "assign",
    "store",
    "receive",
    "subscribe",
    "eraseToAnyPublisher",
    "send",
    "handleEvents",
    "switchToLatest",
    "removeDuplicates",
    "debounce",
    "throttle",
    "delay",
    "combineLatest",
    "prepend",
    "collect",
    "share",
    "replaceError",
    "replaceNil",
    "catch",
    "tryMap",
    "mapError",
    "setFailureType",
    "print",
    "values",
    "publisher",
    "scan",
    "timeout",
    "retry",
    "breakpoint",
    // Concurrency
    "yield",
    "finish",
    "sleep",
    "withTaskGroup",
    "addTask",
    "next",
    // SwiftUI view modifiers
    "frame",
    "padding",
    "background",
    "overlay",
    "foregroundColor",
    "foregroundStyle",
    "font",
    "fontWeight",
    "bold",
    "italic",
    "cornerRadius",
    "clipShape",
    "clipped",
    "shadow",
    "opacity",
    "offset",
    "position",
    "scaleEffect",
    "rotationEffect",
    "onAppear",
    "onDisappear",
    "onTapGesture",
    "onLongPressGesture",
    "onChange",
    "onReceive",
    "task",
    "sheet",
    "fullScreenCover",
    "popover",
    "alert",
    "confirmationDialog",
    "navigationTitle",
    "navigationBarTitleDisplayMode",
    "navigationBarHidden",
    "navigationBarBackButtonHidden",
    "navigationDestination",
    "toolbar",
    "disabled",
    "hidden",
    "id",
    "tag",
    "animation",
    "transition",
    "lineLimit",
    "multilineTextAlignment",
    "fixedSize",
    "layoutPriority",
    "contentShape",
    "allowsHitTesting",
    "accessibilityIdentifier",
    "accessibilityLabel",
    "accessibilityHint",
    "accessibilityValue",
    "accessibilityElement",
    "accessibilityAddTraits",
    "accessibilityHidden",
    "environment",
    "environmentObject",
    "buttonStyle",
    "listRowInsets",
    "listRowBackground",
    "listStyle",
    "textFieldStyle",
    "keyboardType",
    "textContentType",
    "autocapitalization",
    "submitLabel",
    "onSubmit",
    "focused",
    "tint",
    "accentColor",
    "ignoresSafeArea",
    "edgesIgnoringSafeArea",
    "safeAreaInset",
    "gesture",
    "simultaneousGesture",
    "highPriorityGesture",
    "refreshable",
    "searchable",
    "scrollDisabled",
    "scrollIndicators",
    "matchedGeometryEffect",
    "aspectRatio",
    "resizable",
    "renderingMode",
    "interpolation",
    "fill",
    "stroke",
    "strokeBorder",
    "mask",
    "blur",
    "brightness",
    "saturation",
    "grayscale",
    "colorMultiply",
    "blendMode",
    "compositingGroup",
    "drawingGroup",
    "zIndex",
    "border",
    "preference",
    "onPreferenceChange",
    "coordinateSpace",
    "anchorPreference",
    "modifier",
    "labelsHidden",
    "pickerStyle",
    "toggleStyle",
    "progressViewStyle",
    "presentationDetents",
    "interactiveDismissDisabled",
    "redacted",
    "unredacted",
    "minimumScaleFactor",
    "truncationMode",
    "kerning",
    "tracking",
    "baselineOffset",
    "underline",
    "strikethrough",
    "monospacedDigit",
    "dynamicTypeSize",
    "preferredColorScheme",
    "colorScheme",
    "statusBarHidden",
    "onOpenURL",
    "onDrag",
    "onDrop",
    "contextMenu",
    "swipeActions",
    "badge",
    "help",
    "keyboardShortcut",
    "scrollTo",
    "eraseToAnyView",
    "erased",
    // UIKit / Foundation objects
    "addSubview",
    "removeFromSuperview",
    "insertSubview",
    "bringSubviewToFront",
    "layoutIfNeeded",
    "setNeedsLayout",
    "setNeedsDisplay",
    "sizeToFit",
    "present",
    "dismiss",
    "pushViewController",
    "popViewController",
    "popToRootViewController",
    "setViewControllers",
    "addChild",
    "removeFromParent",
    "didMove",
    "willMove",
    "reloadData",
    "reloadSections",
    "reloadRows",
    "register",
    "dequeueReusableCell",
    "dequeueReusableCellWithIdentifier",
    "cellForRow",
    "scrollToRow",
    "deselectRow",
    "selectRow",
    "performBatchUpdates",
    "setTitle",
    "setImage",
    "setTitleColor",
    "addTarget",
    "removeTarget",
    "addGestureRecognizer",
    "becomeFirstResponder",
    "resignFirstResponder",
    "setContentOffset",
    "setValue",
    "setObject",
    "object",
    "removeObject",
    "synchronize",
    "post",
    "addObserver",
    "removeObserver",
    "async",
    "asyncAfter",
    "perform",
    "invalidate",
    "fire",
    "activate",
    "deactivate",
    "constraint",
    "isActive",
    "setContentHuggingPriority",
    "setContentCompressionResistancePriority",
    "dataTask",
    "uploadTask",
    "downloadTask",
    "setBody",
    "addValue",
    "setValueForHTTPHeaderField",
    "jsonObject",
    "log",
    "debug",
    "info",
    "warning",
    "error",
    "fault",
    "notice",
    "trace",
    "critical",
];

/// A project declaration a call may resolve to.
struct Target {
    id: String,
    /// Base name.
    name: String,
    /// Kind label as stored (`function`, `property`, `class`, ...).
    kind: String,
    file: String,
    private: bool,
    /// Parameter labels of a function (`_` = unlabeled), when known.
    labels: Option<Vec<String>>,
    /// Kind of the containing declaration (`protocol`, `extension`, ...).
    container_kind: Option<String>,
    line: u32,
}

/// Outcome of resolving one call site.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// Exactly one plausible target.
    Confident(Vec<String>),
    /// Several plausible targets, at most `max_candidates`.
    Ambiguous(Vec<String>),
    /// No edge; `true` when the name exists in the project.
    Unresolved(bool),
}

/// In-memory index of project declarations for call resolution.
pub(crate) struct Resolver {
    targets: Vec<Target>,
    /// Base name (without argument labels) → members of types, per type name.
    members: HashMap<String, HashMap<String, Vec<usize>>>,
    /// Base name → free functions, globals and top-level types.
    top_level: HashMap<String, Vec<usize>>,
    /// Base name → every member declaration with that name.
    all_members: HashMap<String, Vec<usize>>,
    /// Type name → direct supertypes (protocols, superclasses).
    supertypes: HashMap<String, Vec<String>>,
    /// Every declared base name.
    names: HashSet<String>,
    /// Type name → property → declared type, from every file.
    member_types: HashMap<String, HashMap<String, String>>,
    library: HashSet<&'static str>,
    max_candidates: usize,
    /// Names of protocols declared in the project.
    protocols: HashSet<String>,
}

const TYPE_KINDS: &[&str] = &["class", "struct", "enum", "protocol", "extension"];

/// `update(id:_:)` → `update`.
fn base_name(name: &str) -> &str {
    name.split('(').next().unwrap_or(name)
}

/// `Type.update(id:_:)` → `["id", "_"]`.
fn labels_of(name: &str) -> Option<Vec<String>> {
    let open = name.rfind('(')?;
    let inner = name[open + 1..].strip_suffix(')')?;
    Some(
        inner
            .split(':')
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
    )
}

impl Resolver {
    /// Load every callable or constructible declaration and the type hierarchy.
    pub(crate) fn load(conn: &Connection, config: &ResolutionConfig) -> rusqlite::Result<Self> {
        let mut resolver = Self {
            targets: Vec::new(),
            members: HashMap::new(),
            top_level: HashMap::new(),
            all_members: HashMap::new(),
            supertypes: HashMap::new(),
            names: HashSet::new(),
            member_types: HashMap::new(),
            library: LIBRARY_NAMES.iter().copied().collect(),
            protocols: HashSet::new(),
            max_candidates: config.max_candidates.max(1),
        };

        let mut stmt = conn.prepare(
            "SELECT n.id, n.name, n.qualified_name, n.kind, n.file, n.access_level,
                    c.kind, c.name, n.line
             FROM nodes n LEFT JOIN nodes c ON c.id = n.container_usr
             WHERE n.kind IN ('function', 'method', 'property', 'class', 'struct', 'enum',
                              'protocol', 'typeAlias')
             ORDER BY n.file, n.line, n.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, u32>(8)?,
            ))
        })?;
        for row in rows {
            let (id, name, qualified, kind, file, access, container_kind, container_name, line) =
                row?;
            if kind == "protocol" {
                resolver.protocols.insert(base_name(&name).to_string());
            }
            let base = base_name(&name).to_string();
            let labels = matches!(kind.as_str(), "function" | "method")
                .then(|| {
                    if name.contains('(') {
                        labels_of(&name)
                    } else {
                        labels_of(&qualified)
                    }
                })
                .flatten();
            let idx = resolver.targets.len();
            resolver.targets.push(Target {
                id,
                name: base.clone(),
                kind,
                file,
                private: matches!(access.as_str(), "Private" | "FilePrivate"),
                labels,
                container_kind: container_kind.clone(),
                line,
            });
            resolver.names.insert(base.clone());
            match (container_kind.as_deref(), container_name) {
                (Some(ck), Some(type_name)) if TYPE_KINDS.contains(&ck) => {
                    resolver
                        .members
                        .entry(base_name(&type_name).to_string())
                        .or_default()
                        .entry(base.clone())
                        .or_default()
                        .push(idx);
                    resolver.all_members.entry(base).or_default().push(idx);
                }
                (None, _) => resolver.top_level.entry(base).or_default().push(idx),
                // Declarations nested in functions are not reachable by name.
                _ => {}
            }
        }

        let mut stmt = conn.prepare(
            "SELECT s.name, e.target, t.name
             FROM edges e JOIN nodes s ON s.id = e.source
             LEFT JOIN nodes t ON t.id = e.target
             WHERE e.kind IN ('conformsTo', 'inheritsFrom')
               AND s.kind IN ('class', 'struct', 'enum', 'protocol', 'extension')",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (source, target, target_name) = row?;
            let parent =
                target_name.or_else(|| target.strip_prefix("synthetic::").map(String::from));
            if let Some(parent) = parent {
                let list = resolver.supertypes.entry(source).or_default();
                if !list.contains(&parent) {
                    list.push(parent);
                }
            }
        }
        // `typealias Endpoint = Pinging & Naming`: members of the composed
        // protocols are members of the alias
        let mut stmt =
            conn.prepare("SELECT name, signature FROM nodes WHERE kind = 'typeAlias'")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (alias, signature) = row?;
            let Some(aliased) = signature.as_deref().and_then(|s| s.split_once('=')) else {
                continue;
            };
            let parts: Vec<String> = aliased
                .1
                .split('&')
                .map(|p| {
                    p.trim()
                        .trim_start_matches("any ")
                        .split(['<', ' ', '{'])
                        .next()
                        .unwrap_or("")
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
                .filter(|p| p.chars().next().is_some_and(char::is_uppercase))
                .collect();
            let list = resolver.supertypes.entry(alias).or_default();
            for part in parts {
                if !list.contains(&part) {
                    list.push(part);
                }
            }
        }

        let mut stmt = conn.prepare("SELECT owner, member, type_name FROM member_types")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (owner, member, ty) = row?;
            resolver
                .member_types
                .entry(owner)
                .or_default()
                .insert(member, ty);
        }
        Ok(resolver)
    }

    /// Declared type of property `member` of `owner` or of its nearest
    /// supertype declaring it.
    fn member_type(&self, owner: &str, member: &str) -> Option<&str> {
        let mut seen: HashSet<&str> = HashSet::from([owner]);
        let mut level = vec![owner];
        while !level.is_empty() {
            if let Some(ty) = level
                .iter()
                .find_map(|t| self.member_types.get(*t)?.get(member))
            {
                return Some(ty);
            }
            let mut next = Vec::new();
            for t in level {
                for p in self.parents(t) {
                    if seen.insert(p) {
                        next.push(p);
                    }
                }
            }
            level = next;
        }
        None
    }

    /// Members named `name` of `ty` or, if it has none, of its nearest
    /// supertypes (breadth-first, so overrides hide overridden members).
    fn members_named(&self, ty: &str, name: &str, include_self: bool) -> Vec<usize> {
        let mut seen: HashSet<&str> = HashSet::from([ty]);
        let mut level: Vec<&str> = if include_self {
            vec![ty]
        } else {
            self.parents(ty)
        };
        while !level.is_empty() {
            let found: Vec<usize> = level
                .iter()
                .filter_map(|t| self.members.get(*t)?.get(name))
                .flatten()
                .copied()
                .collect();
            if !found.is_empty() {
                return found;
            }
            let mut next = Vec::new();
            for t in level {
                for p in self.parents(t) {
                    if seen.insert(p) {
                        next.push(p);
                    }
                }
            }
            level = next;
        }
        Vec::new()
    }

    fn parents(&self, ty: &str) -> Vec<&str> {
        self.supertypes
            .get(ty)
            .map(|v| v.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// `(implementation, requirement, file, line)` for every member of a
    /// project type that satisfies a requirement of a project protocol the
    /// type conforms to (directly, in an extension, or through supertypes),
    /// and for default implementations in protocol extensions. Matched by
    /// name and argument labels.
    pub(crate) fn requirement_implementations(&self) -> Vec<(String, String, String, u32)> {
        let mut out = Vec::new();
        let is_requirement =
            |i: &usize| self.targets[*i].container_kind.as_deref() == Some("protocol");
        let same_signature = |a: &Target, b: &Target| a.name == b.name && a.labels == b.labels;
        let mut types: Vec<&String> = self.members.keys().collect();
        types.sort();
        for ty in types {
            let own = &self.members[ty];
            // Protocols reached from this type, including itself for
            // default implementations in its extensions
            let mut seen: HashSet<&str> = HashSet::new();
            let mut stack: Vec<&str> = vec![ty.as_str()];
            let mut protocols: Vec<&str> = Vec::new();
            while let Some(t) = stack.pop() {
                if !seen.insert(t) {
                    continue;
                }
                if self.protocols.contains(t) {
                    protocols.push(t);
                }
                stack.extend(self.parents(t));
            }
            protocols.sort();
            for protocol in protocols {
                let Some(requirements) = self.members.get(protocol) else {
                    continue;
                };
                for (name, reqs) in requirements {
                    let Some(candidates) = own.get(name) else {
                        continue;
                    };
                    for req in reqs.iter().filter(|i| is_requirement(i)) {
                        for imp in candidates.iter().filter(|i| !is_requirement(i)) {
                            let (r, i) = (&self.targets[*req], &self.targets[*imp]);
                            // A protocol's own extension provides defaults only for itself
                            if protocol != ty && i.container_kind.as_deref() == Some("protocol") {
                                continue;
                            }
                            if same_signature(r, i) && r.id != i.id {
                                out.push((i.id.clone(), r.id.clone(), i.file.clone(), i.line));
                            }
                        }
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Whether the call could resolve to project code now or after an edit:
    /// not a standard library name on a receiver of unknown type.
    pub(crate) fn may_resolve(&self, call: &CallSite) -> bool {
        !(matches!(call.receiver, Receiver::Unknown) && self.library.contains(call.name.as_str()))
    }

    /// Whether some project declaration has this base name.
    pub(crate) fn is_declared(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// Whether the call's receiver type stays unknown after looking up
    /// property types across the project.
    pub(crate) fn receiver_unknown(&self, call: &CallSite) -> bool {
        match &call.receiver {
            Receiver::Unknown => true,
            Receiver::Member { owner, member } => self.member_type(owner, member).is_none(),
            _ => false,
        }
    }

    /// Candidates for a call on a receiver of unknown type.
    fn unknown_receiver(&self, name: &str) -> Vec<usize> {
        if self.library.contains(name) {
            return Vec::new();
        }
        self.all_members.get(name).cloned().unwrap_or_default()
    }

    /// Initializers of a type candidate that fit the call's arguments, or
    /// the type itself (implicit memberwise/default initializers).
    fn constructor_targets(&self, idx: usize, call: &CallSite) -> Vec<usize> {
        let target = &self.targets[idx];
        if !matches!(target.kind.as_str(), "class" | "struct" | "enum") {
            return vec![idx];
        }
        let fitting: Vec<usize> = self
            .members
            .get(&target.name)
            .and_then(|m| m.get("init"))
            .into_iter()
            .flatten()
            .copied()
            .filter(|i| argument_fit(&self.targets[*i], call).is_some())
            .collect();
        if fitting.is_empty() {
            vec![idx]
        } else {
            fitting
        }
    }

    /// Resolve one call site.
    pub(crate) fn resolve(&self, call: &CallSite) -> Resolution {
        let name = call.name.as_str();
        let candidates: Vec<usize> = match &call.receiver {
            Receiver::Implicit => call
                .scope
                .iter()
                .map(|t| self.members_named(t, name, true))
                .find(|found| !found.is_empty())
                .unwrap_or_else(|| self.top_level.get(name).cloned().unwrap_or_default()),
            Receiver::Typed(ty) => self.members_named(ty, name, true),
            Receiver::Super(ty) => self.members_named(ty, name, false),
            Receiver::Member { owner, member } => match self.member_type(owner, member) {
                Some(ty) => self.members_named(ty, name, true),
                None => self.unknown_receiver(name),
            },
            Receiver::Unknown => self.unknown_receiver(name),
        };

        // `Type(...)`: the matching initializer when the type declares any
        let candidates: Vec<usize> = candidates
            .into_iter()
            .flat_map(|idx| self.constructor_targets(idx, call))
            .collect();

        let mut best = 0;
        let mut kept: Vec<&str> = Vec::new();
        for idx in candidates {
            let t = &self.targets[idx];
            if t.id == call.caller || (t.private && t.file != call.location.file) {
                continue;
            }
            let Some(score) = argument_fit(t, call) else {
                continue;
            };
            if score > best {
                best = score;
                kept.clear();
            }
            if score == best && !kept.contains(&t.id.as_str()) {
                kept.push(&t.id);
            }
        }

        let ids = || kept.iter().map(|s| s.to_string()).collect();
        match kept.len() {
            0 => Resolution::Unresolved(self.names.contains(name)),
            1 => Resolution::Confident(ids()),
            n if n <= self.max_candidates => Resolution::Ambiguous(ids()),
            _ => Resolution::Unresolved(true),
        }
    }
}

/// How well the call's arguments fit the target: 2 = exact, 1 = plausible
/// (default values, variadics, unknown labels), `None` = impossible.
fn argument_fit(target: &Target, call: &CallSite) -> Option<u8> {
    match target.kind.as_str() {
        "function" | "method" => {
            let Some(params) = &target.labels else {
                return Some(1);
            };
            let mut next = 0;
            let mut exact = true;
            let mut last_unlabeled = false;
            for label in &call.labels {
                let want = label.as_deref().unwrap_or("_");
                match params[next.min(params.len())..]
                    .iter()
                    .position(|p| p == want)
                {
                    Some(k) => {
                        exact &= k == 0;
                        next += k + 1;
                        last_unlabeled = want == "_";
                    }
                    // Further unlabeled arguments of a variadic parameter.
                    None if want == "_" && last_unlabeled => exact = false,
                    None => return None,
                }
            }
            let total = next + call.trailing_closures;
            exact &= total == params.len();
            if total > params.len() && !(last_unlabeled && call.trailing_closures == 0) {
                // Trailing closures need parameters to bind to.
                return None;
            }
            Some(if exact { 2 } else { 1 })
        }
        // Closure-typed properties are called without labels.
        "property" => call.labels.iter().all(Option::is_none).then_some(1),
        // Initializers: labels are not indexed.
        _ => Some(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Location;

    fn call(labels: &[Option<&str>], trailing: usize) -> CallSite {
        CallSite {
            caller: "c".into(),
            name: "f".into(),
            receiver: Receiver::Unknown,
            scope: vec![],
            labels: labels.iter().map(|l| l.map(String::from)).collect(),
            trailing_closures: trailing,
            location: Location {
                file: "A.swift".into(),
                line: 1,
                column: 1,
                end_line: None,
                end_column: None,
            },
        }
    }

    fn func(labels: &[&str]) -> Target {
        Target {
            id: "t".into(),
            name: "f".into(),
            kind: "function".into(),
            file: "A.swift".into(),
            private: false,
            labels: Some(labels.iter().map(|s| s.to_string()).collect()),
            container_kind: None,
            line: 1,
        }
    }

    #[test]
    fn labels_are_parsed_from_swift_names() {
        assert_eq!(
            labels_of("Store.load(id:_:)"),
            Some(vec!["id".to_string(), "_".to_string()])
        );
        assert_eq!(labels_of("load()"), Some(vec![]));
        assert_eq!(labels_of("frame"), None);
        assert_eq!(base_name("load(id:)"), "load");
    }

    #[test]
    fn argument_fit_prefers_exact_label_matches() {
        assert_eq!(
            argument_fit(&func(&["id"]), &call(&[Some("id")], 0)),
            Some(2)
        );
        // Default value for `id`.
        assert_eq!(argument_fit(&func(&["id"]), &call(&[], 0)), Some(1));
        assert_eq!(
            argument_fit(&func(&["model"]), &call(&[Some("id")], 0)),
            None
        );
        // Label required but missing.
        assert_eq!(argument_fit(&func(&["id"]), &call(&[None], 0)), None);
        // Trailing closure binds to the last parameter.
        assert_eq!(
            argument_fit(&func(&["a", "completion"]), &call(&[Some("a")], 1)),
            Some(2)
        );
        assert_eq!(argument_fit(&func(&[]), &call(&[], 1)), None);
        // Variadic `_ items: Any...`.
        assert_eq!(
            argument_fit(&func(&["_"]), &call(&[None, None, None], 0)),
            Some(1)
        );
        let property = Target {
            kind: "property".into(),
            labels: None,
            ..func(&[])
        };
        assert_eq!(argument_fit(&property, &call(&[Some("width")], 0)), None);
        assert_eq!(argument_fit(&property, &call(&[None], 0)), Some(1));
    }
}
