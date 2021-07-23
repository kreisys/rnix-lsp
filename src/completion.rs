use crate::{utils, App};
use itertools::Itertools;
use lsp_types::{
    CompletionItem, CompletionList, CompletionResponse, CompletionTextEdit, Documentation, Range,
    TextDocumentPositionParams, TextEdit,
};
use manix::{DocEntry, DocSource};
use rnix::{
    types::{ParsedType, TokenWrapper, TypedNode},
    NixLanguage, SyntaxKind, SyntaxNode, TextSize,
};
use std::convert::TryFrom;

impl App {
    fn scope_completions(
        &mut self,
        params: &TextDocumentPositionParams,
    ) -> Option<Vec<CompletionItem>> {
        let (ast, content) = self.files.get(&params.text_document.uri)?;
        let offset = utils::lookup_pos(content, params.position)?;
        let root_node = ast.node();

        let (name, scope, _) =
            self.scope_for_ident(params.text_document.uri.clone(), &root_node, offset)?;
        let (_, content) = self.files.get(&params.text_document.uri)?;

        let scope_completions = scope
            .keys()
            .filter(|var| var.starts_with(&name.as_str()))
            .map(|var| CompletionItem {
                label: var.clone(),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: utils::range(content, name.node().text_range()),
                    new_text: var.clone(),
                })),
                ..CompletionItem::default()
            })
            .collect_vec();
        Some(scope_completions)
    }

    fn manix_options_completions(
        &self,
        params: &TextDocumentPositionParams,
    ) -> Option<Vec<CompletionItem>> {
        // TODO implement this
        None
    }

    fn manix_value_completions(
        &self,
        params: &TextDocumentPositionParams,
    ) -> Option<Vec<CompletionItem>> {
        let (ast, content) = self.files.get(&params.text_document.uri)?;
        let offset = utils::lookup_pos(content, params.position)?;
        let root_node = ast.node();

        let node = utils::closest_node_to(&root_node, offset)?;
        let (full_ident_node, full_ident_name) = self.full_ident_name(&node)?;
        dbg!(node.text_range());

        let node_range = Range {
            start: utils::offset_to_pos(
                content,
                full_ident_node
                    .first_token()?
                    .text_range()
                    .start()
                    .into(),
            ),
            end: utils::offset_to_pos(
                content,
                full_ident_node
                    .descendants_with_tokens()
                    .take_while(|n| match n {
                        rnix::NodeOrToken::Node(_) => true,
                        rnix::NodeOrToken::Token(t) => {
                            t.kind() == SyntaxKind::TOKEN_DOT || t.kind() == SyntaxKind::TOKEN_IDENT
                        }
                    })
                    .last()?
                    .text_range()
                    .end()
                    .into(),
            ),
        };

        let search_results = self.manix_values.search(&manix::Lowercase(
            &full_ident_name.clone().join(".").as_bytes(),
        ));

        let (namespace, namespace_items) =
            self.next_namespace_step_completions(full_ident_name.clone(), search_results);

        let manix_completions = namespace_items
            .iter()
            .unique_by(|x| x.name())
            .map(|def| CompletionItem {
                label: def.name().clone(),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: node_range,
                    new_text: def.name().clone(),
                })),
                documentation: def
                    .try_as_doc_entry()
                    .map(|entry| Documentation::String(entry.pretty_printed())),
                ..CompletionItem::default()
            })
            .collect_vec();
        Some(manix_completions)
    }

    #[allow(clippy::shadow_unrelated)] // false positive
    pub fn completions(
        &mut self,
        params: &TextDocumentPositionParams,
    ) -> Option<Vec<CompletionItem>> {
        // let scope_completions = self.scope_completions(params)?;
        let mut manix_value_completions = self.manix_value_completions(params).unwrap_or_default();
        let mut manix_options_completions =
            self.manix_options_completions(params).unwrap_or_default();
        let mut completions = Vec::new();
        completions.append(&mut manix_value_completions);
        completions.append(&mut manix_options_completions);

        Some(completions)
    }

    fn next_namespace_step_completions(
        &self,
        current_ns: Vec<String>,
        search_results: Vec<DocEntry>,
    ) -> (Vec<String>, Vec<NamespaceCompletionResult>) {
        // TODO handle things like `with pkgs;`

        let query_ns_iter = current_ns.iter();
        let longest_match = search_results
            .iter()
            .map(|result| {
                result
                    .name()
                    .split('.')
                    .zip(query_ns_iter.clone())
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| a.to_string())
                    .collect_vec()
            })
            .max();
        if let Some(longest_match) = longest_match {
            dbg!(&current_ns, &longest_match);
            let completions = search_results
                .into_iter()
                .filter(|result| {
                    result
                        .name()
                        .split('.')
                        .zip(query_ns_iter.clone())
                        .take_while(|(a, b)| a == b)
                        .count()
                        > 0
                })
                .map(|result| {
                    use NamespaceCompletionResult::*;
                    if result.name().split('.').count() - 1 == longest_match.len() {
                        FinalNode(result)
                    } else {
                        let presented_result =
                            result.name().split('.').take(longest_match.len()).join(".");
                        Set(presented_result)
                    }
                })
                .unique_by(|x| x.name())
                .collect_vec();
            (current_ns, completions)
        } else {
            (current_ns, Vec::new())
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum NamespaceCompletionResult {
    Set(String),
    FinalNode(DocEntry),
}

impl NamespaceCompletionResult {
    fn name(&self) -> String {
        use NamespaceCompletionResult::*;
        match self {
            Set(s) => s.to_owned(),
            FinalNode(entry) => entry.name(),
        }
    }

    fn try_as_doc_entry(&self) -> Option<&DocEntry> {
        use NamespaceCompletionResult::*;
        match self {
            Set(_) => None,
            FinalNode(entry) => Some(entry),
        }
    }
}
