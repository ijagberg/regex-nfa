use crate::automaton::Automaton;
use regex_syntax::ast::parse::Parser;
use regex_syntax::ast::{
    Alternation, Ast, Class, ClassSet, ClassSetItem, ClassSetRange, Concat, Error, Repetition,
};
use std::collections::HashSet;

type TranslatorResult = Result<Automaton, TranslatorError>;

#[derive(Debug)]
pub enum TranslatorError {
    UnsupportedAst(Ast),
    UnsupportedClass(Class),
    UnsupportedClassSet(ClassSet),
    UnsupportedClassSetItem(ClassSetItem),
    ParserError(Error),
}

impl std::fmt::Display for TranslatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "error when translating regular expression: {}",
            match self {
                TranslatorError::UnsupportedAst(ast) => {
                    format!("{:?} is not a supported ast kind (yet)", ast)
                }
                TranslatorError::UnsupportedClass(class) => {
                    format!("{:?} is not a supported class kind (yet)", class)
                }
                TranslatorError::UnsupportedClassSet(class_set) => {
                    format!("{:?} is not a supported class set kind (yet)", class_set)
                }
                TranslatorError::UnsupportedClassSetItem(class_set_item) => format!(
                    "{:?} is not a supported class set item kind (yet)",
                    class_set_item
                ),
                TranslatorError::ParserError(parser_error) => parser_error.to_string(),
            }
        )
    }
}

impl std::error::Error for TranslatorError {}

pub(crate) fn translate(s: &str) -> TranslatorResult {
    match Parser::new().parse(s) {
        Ok(ast) => build_tree(&ast),
        Err(err) => Err(TranslatorError::ParserError(err)),
    }
}

fn build_tree(ast_tree: &Ast) -> TranslatorResult {
    match ast_tree {
        Ast::Concat(ast) => build_concatenation(ast),
        Ast::Repetition(ast) => build_repetition(ast),
        Ast::Literal(ast) => build_literal(std::iter::once(ast.c).collect()),
        Ast::Alternation(ast) => build_alternation(ast),
        Ast::Group(ast) => build_tree(&ast.ast),
        Ast::Class(ast) => build_class(ast),
        unsupported => Err(TranslatorError::UnsupportedAst(unsupported.clone())),
    }
}

fn build_class(class_ast: &Class) -> TranslatorResult {
    match class_ast {
        Class::Bracketed(class_bracketed) => match &class_bracketed.kind {
            ClassSet::Item(item) => match item {
                ClassSetItem::Range(class_set_range) => build_class_set_range(&class_set_range),
                unsupported => Err(TranslatorError::UnsupportedClassSetItem(
                    unsupported.clone(),
                )),
            },
            unsupported => Err(TranslatorError::UnsupportedClassSet(unsupported.clone())),
        },
        unsupported => Err(TranslatorError::UnsupportedClass(unsupported.clone())),
    }
}

/// Builds an automaton simulating a regular expression like ```[a-z]```
/// by just building a literal with each symbol in the range as its transition
fn build_class_set_range(class_set_range: &ClassSetRange) -> TranslatorResult {
    let start_atom = class_set_range.start.c as u8;
    let end_atom = class_set_range.end.c as u8;

    build_literal(((start_atom..=end_atom).map(char::from)).collect())
}

/// Builds an automaton simulating a regular expression like ```abc```
/// by appending each symbol to the end state of the previous symbol, a -> b -> _c_
fn build_concatenation(concat_ast: &Concat) -> TranslatorResult {
    let mut concat_automaton = Automaton::new();
    let concat_start_state = concat_automaton.add_state();
    concat_automaton.set_start_state(concat_start_state);

    let mut concat_end_state = concat_start_state;

    for append_ast in &concat_ast.asts {
        let append_automaton = build_tree(append_ast)?;
        assert_eq!(append_automaton.accepting_states.len(), 1);
        let append_start_state = append_automaton.start_state;
        let append_end_state = *append_automaton.accepting_states.iter().next().unwrap();
        let concat_append_offset = concat_automaton.states;
        concat_automaton.add_states_and_transitions(append_automaton);

        // Add transition from previous append_automaton's end state to current append_automaton's start state
        concat_automaton.add_transition(
            concat_end_state,
            append_start_state + concat_append_offset,
            None,
        );

        // Change end state to be the current append_automaton's end state
        concat_end_state = append_end_state + concat_append_offset;
        concat_automaton.clear_accepting();
        concat_automaton.set_accepting(concat_end_state, true);
    }

    Ok(concat_automaton)
}

/// Builds an automaton simulating a regular expression like ```a?```, ```a+``` or ```a*```
/// For ```?```, create two states with the repeating automaton between them, and add an epsilon
/// transition from the starting state to the end (accepting) state.
/// For ```+```, create two states with the repeating automaton between them, and add an epsilon
/// transition from the end (accepting) state to the starting state.
/// For ```*```, create two states with the repeating automaton between them, and add an epsilon
/// transition from the starting state to the end (accepting) state, and an epsilon transition from
/// the end (accepting) state to the starting state.
fn build_repetition(repetition_ast: &Repetition) -> TranslatorResult {
    use regex_syntax::ast::RepetitionKind;

    let mut repetition_automaton = Automaton::new();
    let repetition_start_state = repetition_automaton.add_state();
    let repetition_end_state = repetition_automaton.add_state();
    let repetition_to_inner_offset = repetition_automaton.states;

    let inner_automaton = build_tree(&repetition_ast.ast)?;
    assert_eq!(inner_automaton.accepting_states.len(), 1);
    let inner_automaton_start_state = inner_automaton.start_state;
    let inner_automaton_end_state = *inner_automaton.accepting_states.iter().next().unwrap();
    repetition_automaton.add_states_and_transitions(inner_automaton);

    // Add transition from repetition_automaton's start state to inner_automaton's start state
    repetition_automaton.add_transition(
        repetition_start_state,
        inner_automaton_start_state + repetition_to_inner_offset,
        None,
    );

    // Add transition from inner_automaton's end state to repetition_automaton's end state
    repetition_automaton.add_transition(
        inner_automaton_end_state + repetition_to_inner_offset,
        repetition_end_state,
        None,
    );

    match &repetition_ast.op.kind {
        RepetitionKind::OneOrMore => {
            // Add transition from repetition_automaton's end state to repetition_automaton's start state
            repetition_automaton.add_transition(repetition_end_state, repetition_start_state, None);
        }
        RepetitionKind::ZeroOrMore => {
            // Add transition from repetition_automaton's start state to repetition_automaton's end state (for Zero)
            repetition_automaton.add_transition(repetition_start_state, repetition_end_state, None);
            // Add transition from repetition_automaton's end state to repetition_automaton's start state
            repetition_automaton.add_transition(repetition_end_state, repetition_start_state, None);
        }
        RepetitionKind::ZeroOrOne => {
            // Add transition from repetition_automaton's start state to repetition_automaton's end state
            repetition_automaton.add_transition(repetition_start_state, repetition_end_state, None);
        }
        unsupported => {
            panic!("{:?} is not supported yet", unsupported);
        }
    }

    repetition_automaton.set_start_state(repetition_start_state);
    repetition_automaton.clear_accepting();
    repetition_automaton.set_accepting(repetition_end_state, true);

    Ok(repetition_automaton)
}

fn build_alternation(alternation_ast: &Alternation) -> TranslatorResult {
    let mut alternation_automaton = Automaton::new();
    let alternation_automaton_start_state = alternation_automaton.add_state();
    let alternation_automaton_end_state = alternation_automaton.add_state();

    for alternative_ast in &alternation_ast.asts {
        let alternative_automaton = build_tree(alternative_ast)?;
        assert_eq!(alternative_automaton.accepting_states.len(), 1);

        let alternative_automaton_start_state = alternative_automaton.start_state;
        let alternative_automaton_end_state = *alternative_automaton
            .accepting_states
            .iter()
            .next()
            .unwrap();
        let alternation_to_alternative_offset = alternation_automaton.states;
        alternation_automaton.add_states_and_transitions(alternative_automaton);

        // Add transition from alternation_automaton's start state to alternative_automaton's start state
        alternation_automaton.add_transition(
            alternation_automaton_start_state,
            alternative_automaton_start_state + alternation_to_alternative_offset,
            None,
        );

        // Add transition from alternative_automaton's end state to alternation_automaton's end state
        alternation_automaton.add_transition(
            alternative_automaton_end_state + alternation_to_alternative_offset,
            alternation_automaton_end_state,
            None,
        );
    }

    alternation_automaton.set_start_state(alternation_automaton_start_state);
    alternation_automaton.clear_accepting();
    alternation_automaton.set_accepting(alternation_automaton_end_state, true);

    Ok(alternation_automaton)
}

fn build_literal(atoms: HashSet<char>) -> TranslatorResult {
    let mut literal_automaton = Automaton::new();
    let start_state = literal_automaton.add_state();
    let end_state = literal_automaton.add_state();
    literal_automaton.set_accepting(end_state, true);
    literal_automaton.set_start_state(start_state);
    for atom in atoms {
        literal_automaton.add_transition(start_state, end_state, Some(atom));
    }
    Ok(literal_automaton)
}
