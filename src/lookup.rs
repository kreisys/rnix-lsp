use crate::{
    utils::{self, Datatype, Var},
    App,
};
use lsp_types::Url;
use rnix::{types::*, value::Value as ParsedValue, NodeOrToken, SyntaxKind, SyntaxNode};
use std::{
    collections::{hash_map::Entry, HashMap},
    convert::TryFrom,
    fs,
    rc::Rc,
};

use lazy_static::lazy_static;

use std::{process, str};
use regex;

lazy_static! {
    static ref BUILTINS: Vec<String> = vec![
      // `nix __dump-builtins | jq 'keys'
      "abort", "add", "all", "any", "attrNames", "attrValues", "baseNameOf", "bitAnd", "bitOr",
      "bitXor", "catAttrs", "compareVersions", "concatLists", "concatMap", "concatStringsSep", "deepSeq",
      "dirOf", "div", "elem", "elemAt", "fetchGit", "fetchTarball", "fetchurl", "filter", "filterSource", "foldl'",
      "fromJSON", "functionArgs", "genList", "getAttr", "getEnv", "hasAttr", "hashFile", "hashString", "head",
      "import", "intersectAttrs", "isAttrs", "isBool", "isFloat", "isFunction", "isInt", "isList", "isNull",
      "isPath", "isString", "length", "lessThan", "listToAttrs", "map", "mapAttrs", "match", "mul", "parseDrvName",
      "partition", "path", "pathExists", "placeholder", "readDir", "readFile", "removeAttrs", "replaceStrings",
      "seq", "sort", "split", "splitVersion", "storePath", "stringLength", "sub", "substring", "tail", "throw",
      "toFile", "toJSON", "toPath", "toString", "toXML", "trace", "tryEval", "typeOf"
    ].into_iter().map(String::from).collect::<Vec<_>>();
}

#[derive(Debug)]
pub struct LSPDetails {
    pub datatype: Datatype,
    pub var: Option<Var>,
    pub documentation: Option<String>,
    pub deprecated: bool,
    pub params: Option<String>,
}

impl LSPDetails {
    fn builtin_fallback() -> LSPDetails {
        LSPDetails {
            datatype: Datatype::Lambda,
            var: None,
            documentation: None,
            deprecated: false,
            params: None,
        }
    }

    fn builtin_with_doc(deprecated: bool, params: Option<String>, documentation: String) -> LSPDetails {
        LSPDetails {
            datatype: Datatype::Lambda,
            var: None,
            documentation: Some(documentation),
            deprecated,
            params,
        }
    }

    fn from_scope(datatype: Datatype, var: Var) -> LSPDetails {
        LSPDetails {
            datatype,
            var: Some(var),
            documentation: None,
            deprecated: false,
            params: None,
        }
    }

    pub fn render_detail(&self) -> String {
        match &self.params {
            None => self.datatype.to_string(),
            Some(params) => format!("{}: {} -> Result", self.datatype.to_string(), params),
        }
    }
}

impl App {
    pub fn scope_for_ident(
        &mut self,
        file: Url,
        root: &SyntaxNode,
        offset: usize,
    ) -> Option<(Ident, HashMap<String, LSPDetails>, String)> {

        let mut file = Rc::new(file);
        let info = utils::ident_at(&root, offset)?;
        let ident = info.ident;
        let mut entries = utils::scope_for(&file, ident.node().clone())?
            .into_iter()
            .map(|(x, var)| (x.to_owned(), LSPDetails::from_scope(var.datatype, var)))
            .collect::<HashMap<_, _>>();
        for var in info.path {
            if !entries.contains_key(&var) && var == "builtins" {
                entries = self.load_builtins();
            } else {
                let node_entry = entries.get(&var)?;
                if let Some(var) = &node_entry.var {
                    let node = var.value.clone()?;
                    entries = self
                        .scope_from_node(&mut file, node)?
                        .into_iter()
                        .map(|(x, var)| (x.to_owned(), LSPDetails::from_scope(var.datatype, var)))
                        .collect::<HashMap<_, _>>();
                }
            }
        }
        Some((
            Ident::cast(ident.node().clone()).unwrap(),
            entries,
            info.name,
        ))
    }
    pub fn scope_from_node(
        &mut self,
        file: &mut Rc<Url>,
        mut node: SyntaxNode,
    ) -> Option<HashMap<String, Var>> {
        let mut scope = HashMap::new();

        if let Some(entry) = KeyValue::cast(node.clone()) {
            node = entry.value()?;
        }

        // Resolve simple imports
        loop {
            let apply = match Apply::cast(node.clone()) {
                None => break,
                Some(apply) => apply,
            };
            if Ident::cast(apply.lambda()?).map_or(true, |ident| ident.as_str() != "import") {
                break;
            }
            let (_anchor, path) = match Value::cast(apply.value()?) {
                None => break,
                Some(value) => match value.to_value() {
                    Ok(ParsedValue::Path(anchor, path)) => (anchor, path),
                    _ => break,
                },
            };

            // TODO use anchor
            *file = Rc::new(file.join(&path).ok()?);
            let path = utils::uri_path(&file)?;
            node = match self.files.entry((**file).clone()) {
                Entry::Occupied(entry) => {
                    let (ast, _code) = entry.get();
                    ast.root().inner()?.clone()
                }
                Entry::Vacant(placeholder) => {
                    let content = fs::read_to_string(&path).ok()?;
                    let ast = rnix::parse(&content);
                    let node = ast.root().inner()?.clone();
                    placeholder.insert((ast, content));
                    node
                }
            };
        }

        if let Some(set) = AttrSet::cast(node) {
            utils::populate(&file, &mut scope, &set, Datatype::Attribute);
        }
        Some(scope)
    }

    pub fn full_ident_name(&self, node: &SyntaxNode) -> Option<(SyntaxNode, Vec<String>)> {
        let try_get_ident_name = |x: SyntaxNode| match ParsedType::try_from(x) {
            Ok(ParsedType::Ident(ident)) => Some(ident.as_str().to_string()),
            _ => None,
        };

        let node_path_pair: Option<(SyntaxNode, Vec<String>)> = node.ancestors().find_map(|node| {
            let path = match ParsedType::try_from(node.clone()) {
                Ok(ParsedType::Key(key)) => {
                    let path = key
                        .node()
                        .children_with_tokens()
                        .take_while(|n| match n {
                            NodeOrToken::Node(n) => n.kind() == SyntaxKind::NODE_IDENT,
                            NodeOrToken::Token(t) => t.kind() == SyntaxKind::TOKEN_DOT,
                        })
                        .filter_map(|n| n.as_node().cloned())
                        .filter_map(try_get_ident_name)
                        .filter(|name| !name.trim().trim_end_matches("\n").is_empty())
                        .map(|x| x.replace("\n", ""))
                        .collect::<Vec<_>>();
                    Some(path)
                }
                _ => None,
            };
            path.map(|x| (node, x))
        });

        let node_path_pair = node_path_pair.or_else(|| {
            let mut outermost_select = None;
            for ancestor in node.ancestors() {
                match ParsedType::try_from(ancestor.clone()) {
                    Ok(ParsedType::Select(select)) => {
                        outermost_select = Some(select);
                    }
                    _ if outermost_select.is_some() => {
                        break;
                    }
                    _ => {}
                }
            }

            let mut path = Vec::new();
            for child in outermost_select.clone()?.node().descendants_with_tokens() {
                match child {
                    NodeOrToken::Node(_) => {}
                    NodeOrToken::Token(t) if t.kind() == SyntaxKind::TOKEN_DOT => {}
                    NodeOrToken::Token(t) if t.kind() == SyntaxKind::TOKEN_IDENT => {
                        path.push(t.text().to_string());
                    }
                    NodeOrToken::Token(_) => {
                        break;
                    }
                }
            }
            Some((outermost_select?.node().clone(), path))
        });

        // Ok(ParsedType::Select(key)) => {
        //     let path = key
        //         .node()
        //         .children_with_tokens()
        //         .take_while(|n| match n {
        //             NodeOrToken::Node(n) => n.kind() == SyntaxKind::NODE_IDENT,
        //             NodeOrToken::Token(t) => t.kind() == SyntaxKind::TOKEN_DOT,
        //         })
        //         .filter_map(|n| n.as_node().cloned())
        //         .filter_map(try_get_ident_name)
        //         .filter(|name| !name.trim().trim_end_matches("\n").is_empty())
        //         .map(|x| x.replace("\n", ""))
        //         .collect::<Vec<_>>();
        //     Some(path)
        // }
        dbg!(&node_path_pair);

        Some(node_path_pair?)
    }

    pub fn namespace_for_node(&self, node: &SyntaxNode) -> Vec<String> {
        let mut path = node
            .parent()
            .map(|p| self.namespace_for_node(&p))
            .unwrap_or_default();

        if let Ok(ParsedType::KeyValue(key_value)) = ParsedType::try_from(node.clone()) {
            let mut my_path = key_value
                .key()
                .unwrap()
                .path()
                .map(|x| x.to_string())
                .collect::<Vec<_>>();
            path.append(&mut my_path);
        }
        path
    }

    fn fallback_builtins(&self, list: Vec<String>) -> HashMap<String, LSPDetails> {
        list.into_iter().map(|x| (x, LSPDetails::builtin_fallback())).collect::<HashMap<_, _>>()
    }

    fn load_builtins(&self) -> HashMap<String, LSPDetails> {
        let nixver = process::Command::new("nix").args(&["--version"]).output();

        // `nix __dump-builtins` is only supported on `nixUnstable` a.k.a. Nix 2.4.
        // Thus, we have to check if this is actually available. If not, `rnix-lsp` will fall
        // back to a hard-coded list of builtins which is missing additional info such as documentation
        // or parameter names though.
        match nixver {
            Ok(out) => {
                match str::from_utf8(&out.stdout) {
                    Ok(v) => {
                        let re = regex::Regex::new(r"^nix \(Nix\) (?P<major>\d)\.(?P<minor>\d).*").unwrap();
                        let m = re.captures(v).unwrap();
                        let major = m.name("major").map_or(1, |m| m.as_str().parse::<u8>().unwrap());
                        let minor = m.name("minor").map_or(1, |m| m.as_str().parse::<u8>().unwrap());
                        if major == 2 && minor >= 4 || major > 2 {
                            let builtins_raw = process::Command::new("nix").args(&["__dump-builtins"]).output().unwrap();
                            let v: serde_json::Value = serde_json::from_str(str::from_utf8(&builtins_raw.stdout).unwrap()).unwrap();

                            v.as_object().unwrap()
                                .iter().map(|(x, v)| {
                                    let doc = String::from(v["doc"].as_str().unwrap());
                                    (String::from(x), LSPDetails::builtin_with_doc(
                                        doc.starts_with("**DEPRECATED.**"),
                                        // FIXME make sure that `lib.flip` is taken into account here
                                        v["args"].as_array().map(|x| x.iter().map(|y| y.as_str().unwrap()).collect::<Vec<_>>().join(" -> ")),
                                        doc
                                    ))
                                })
                                .collect::<HashMap<_, _>>()
                        } else {
                            self.fallback_builtins(BUILTINS.to_vec())
                        }
                    },
                    Err(_) => self.fallback_builtins(BUILTINS.to_vec()),
                }
            },
            Err(_) => self.fallback_builtins(BUILTINS.to_vec()),
        }
    }
}
