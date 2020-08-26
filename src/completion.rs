use crate::{utils, App};
use itertools::Itertools;
use lsp_types::{
    CompletionItem, CompletionList, CompletionResponse, CompletionTextEdit, Documentation,
    TextDocumentPositionParams, TextEdit,
};
use manix::{DocEntry, DocSource};
use rnix::{
    types::{TokenWrapper, TypedNode},
    NixLanguage, SyntaxNode, TextUnit,
};

impl App {
    fn scope_completions(
        &mut self,
        params: &TextDocumentPositionParams,
    ) -> Option<Vec<CompletionItem>> {
        let (ast, content) = self.files.get(&params.text_document.uri)?;
        let offset = utils::lookup_pos(content, params.position)?;
        let root_node = ast.node();

        let (name, scope) =
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

    fn manix_completions(&self, params: &TextDocumentPositionParams) -> Option<CompletionResponse> {
        let (ast, content) = self.files.get(&params.text_document.uri)?;
        let offset = utils::lookup_pos(content, params.position)?;
        let root_node = ast.node();

        let node = utils::closest_node_to(&root_node, offset)?;
        let (full_ident_node, full_ident_name) = self.full_ident_name(&node)?;
        dbg!(node.text_range());
        dbg!(full_ident_node.text_range());

        let (namespace, namespace_items) = self.next_namespace_step_completions(full_ident_name);

        let manix_completions = namespace_items
            .iter()
            .unique_by(|x| x.name())
            .map(|def| {
                // let text_to_complete = def
                //     .trim_start_matches(&(namespace.clone() + "."))
                //     .to_owned();
                let text_to_complete = def.name().to_owned();

                CompletionItem {
                    label: text_to_complete.clone(),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: utils::range(content, full_ident_node.text_range()),
                        new_text: text_to_complete,
                    })),
                    documentation: def
                        .try_as_doc_entry()
                        .map(|entry| Documentation::String(entry.pretty_printed())),
                    ..CompletionItem::default()
                }
            })
            .collect_vec();
        Some(CompletionResponse::List(CompletionList {
            is_incomplete: true,
            items: manix_completions,
        }))
    }

    #[allow(clippy::shadow_unrelated)] // false positive
    pub fn completions(
        &mut self,
        params: &TextDocumentPositionParams,
    ) -> Option<CompletionResponse> {
        // let scope_completions = self.scope_completions(params)?;
        let manix_completions = self.manix_completions(params)?;
        Some(manix_completions)
    }

    fn next_namespace_step_completions(
        &self,
        current_ns: Vec<String>,
    ) -> (String, Vec<NamespaceCompletionResult>) {
        let results = self.manix_source.search(&current_ns.join("."));

        // while let Some((_, tail)) = current_ns.split_first() {
        //     if !results.is_empty() {
        //         break;
        //     }
        //     current_ns = tail.to_vec();
        //     results = self.manix_source.search(&current_ns.join("."));
        // }

        let query_ns_iter = current_ns.iter();
        let longest_match = results
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
            let completions = results
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
                    if result.name().replace("\n", "").split('.').count() - 1 == longest_match.len()
                    {
                        FinalNode(result)
                    } else {
                        Set(result
                            .name()
                            .replace("\n", "")
                            .split('.')
                            .take(longest_match.len())
                            .join("."))
                    }
                })
                .unique_by(|x| x.name())
                .collect_vec();
            (current_ns.join("."), completions)
        } else {
            (current_ns.join("."), Vec::new())
        }

        // let first_result_name = if let Some(first_result) = results.first() {
        //     first_result.name().clone()
        // } else {
        //     return (current_ns.join("."), Vec::new());
        // };

        // let longest_match = first_result_name
        //     .split(".")
        //     .enumerate()
        //     .take_while(|(i, part)| Some(*part) == current_ns.get(*i).map(|x| x.as_str()));
        // let longest_match_len = longest_match.count();

        // let completions = results
        //     .iter()
        //     .map(|res| {
        //         dbg!(longest_match_len);
        //         dbg!(res.name().split('.').count());
        //         // if res.name().split('.').count() == longest_match_len {
        //         // }
        //         // res.name().split('.').take(longest_match_len + 1).join(".")
        //     })
        //     .dedup()
        //     .collect_vec();
        // (current_ns.join("."), completions)
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
