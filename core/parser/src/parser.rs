#![cfg_attr(feature = "panic_on_parser_error", allow(unreachable_code))]

use crate::{
    constant_pool::ConstantPoolBuilder,
    error::{ExpectedIndentation, InternalError, ParserError, ParserErrorKind, SyntaxError},
    *,
};
use koto_lexer::{Lexer, Span, Token};
use std::{collections::HashSet, str::FromStr};

// Contains info about the current frame, representing either the module's top level or a function
#[derive(Debug, Default)]
struct Frame {
    // If a frame contains yield then it represents a generator function
    contains_yield: bool,
    // IDs that have been assigned within the current frame
    ids_assigned_in_frame: HashSet<u32>,
    // IDs and lookup roots which were accessed when not locally assigned at the time of access
    accessed_non_locals: HashSet<u32>,
    // While expressions are being parsed we keep track of lhs assignments and rhs accesses.
    // At the end of a multi-assignment expresson (see `finalize_id_accesses`),
    // accessed IDs that weren't locally assigned at the time of access are then counted as
    // non-local accesses.
    pending_accesses: HashSet<u32>,
    pending_assignments: HashSet<u32>,
}

impl Frame {
    // The number of local values declared within the frame
    fn local_count(&self) -> usize {
        self.ids_assigned_in_frame.len()
    }

    // Non-locals accessed in a nested frame need to be declared as also accessed in this
    // frame. This ensures that captures from the outer frame will be available when
    // creating the nested inner frame.
    fn add_nested_accessed_non_locals(&mut self, nested_frame: &Frame) {
        for non_local in nested_frame.accessed_non_locals.iter() {
            if !self.pending_assignments.contains(non_local) {
                self.add_id_access(*non_local);
            }
        }
    }

    // Declare that an id has been accessed within the frame
    fn add_id_access(&mut self, id: u32) {
        self.pending_accesses.insert(id);
    }

    // Declare that an id is being assigned to within the frame
    fn add_local_id_assignment(&mut self, id: u32) {
        self.pending_assignments.insert(id);
        // While an assignment expression is being parsed, the LHS id is counted as an access
        // until the assignment operator is encountered.
        self.pending_accesses.remove(&id);
    }

    // At the end of an expression, determine which RHS accesses are non-local
    fn finalize_id_accesses(&mut self) {
        for id in self.pending_accesses.drain() {
            if !self.ids_assigned_in_frame.contains(&id) {
                self.accessed_non_locals.insert(id);
            }
        }

        self.ids_assigned_in_frame
            .extend(self.pending_assignments.drain());
    }
}

// The set of rules that can modify how an expression is parsed
#[derive(Clone, Copy, Debug)]
struct ExpressionContext {
    // e.g.
    //
    // match x
    //   foo.bar if x == 0 then...
    //
    // Without the flag, `if f == 0...` would be parsed as being an argument for a call to foo.bar.
    allow_space_separated_call: bool,
    // e.g. f = |x|
    //        x + x
    // This function can have an indented body.
    //
    // foo
    //   bar,
    //   baz
    // This function call can be broken over lines.
    //
    // while x < f y
    //   ...
    // Here, `f y` can't be broken over lines as the while expression expects an indented block.
    allow_linebreaks: bool,
    // When true, a map block is allowed in the current context.
    // e.g.
    //
    // x = foo: 42
    //        ^~~ A map block requires an indented block, so here the flag should be false
    //
    // return
    //   foo: 41
    //      ^~~ A colon following the foo identifier signifies the start of a map block.
    //          Consuming tokens through the indentation sets the flag to true,
    //          see consume_until_next_token()
    //
    // x = ||
    //   foo: 42
    //      ^~~ The first line in an indented block will have the flag set to true to allow the
    //          block to be parsed as a map, see parse_indented_block().
    allow_map_block: bool,
    // The indentation rules for the current context
    expected_indentation: Indentation,
}

// The indentation that should be expected on following lines for an expression to continue
#[derive(Clone, Copy, Debug)]
enum Indentation {
    // Indentation isn't required
    // (e.g. in a comma separated braced expression)
    Flexible,
    // Indentation should match the expected indentation
    // (e.g. in an indented block, each line should start with the same indentation)
    Equal(usize),
    // Indentation should be greater than the current indentation
    Greater,
    // Indentation should be greater than the specified indentation
    GreaterThan(usize),
    // Indentation should be greater than or equal to the specified indentation
    GreaterOrEqual(usize),
}

impl ExpressionContext {
    fn permissive() -> Self {
        Self {
            allow_space_separated_call: true,
            allow_linebreaks: true,
            allow_map_block: false,
            expected_indentation: Indentation::Greater,
        }
    }

    fn restricted() -> Self {
        Self {
            allow_space_separated_call: false,
            allow_linebreaks: false,
            allow_map_block: false,
            expected_indentation: Indentation::Greater,
        }
    }

    fn inline() -> Self {
        Self {
            allow_space_separated_call: true,
            allow_linebreaks: false,
            allow_map_block: false,
            expected_indentation: Indentation::Greater,
        }
    }

    // After a keyword like `yield` or `return`.
    // Like inline(), but inherits allow_linebreaks
    fn start_new_expression(&self) -> Self {
        Self {
            allow_space_separated_call: true,
            allow_linebreaks: self.allow_linebreaks,
            allow_map_block: false,
            expected_indentation: Indentation::Greater,
        }
    }

    // At the start of a braced expression
    // e.g.
    //   x = [f x, y] # A single entry list is created with the result of calling `f(x, y)`
    fn braced_items_start() -> Self {
        Self {
            allow_space_separated_call: true,
            allow_linebreaks: true,
            allow_map_block: false,
            expected_indentation: Indentation::Flexible,
        }
    }

    // After the first item in a braced expression
    // Space-separated calls aren't allowed after the first entry,
    // otherwise confusing expressions like the following would be accepted:
    //   x = [1, 2, foo 3, 4, 5]
    //   # This would be parsed as [1, 2, foo(3, 4, 5)]
    fn braced_items_continued() -> Self {
        Self {
            allow_space_separated_call: false,
            allow_linebreaks: true,
            allow_map_block: false,
            expected_indentation: Indentation::Flexible,
        }
    }

    // e.g.
    // [
    //   foo
    //     .bar()
    // # ^ here we're allowing an indented lookup to be started
    // ]
    fn lookup_start(&self) -> Self {
        use Indentation::*;

        let expected_indentation = match self.expected_indentation {
            Flexible | Equal(_) => Greater,
            other => other,
        };

        Self {
            allow_space_separated_call: self.allow_space_separated_call,
            allow_linebreaks: self.allow_linebreaks,
            allow_map_block: false,
            expected_indentation,
        }
    }

    fn with_expected_indentation(&self, expected_indentation: Indentation) -> Self {
        Self {
            expected_indentation,
            ..*self
        }
    }
}

/// Koto's parser
pub struct Parser<'source> {
    ast: Ast,
    constants: ConstantPoolBuilder,
    lexer: Lexer<'source>,
    frame_stack: Vec<Frame>,
}

impl<'source> Parser<'source> {
    /// Takes in a source script, and produces an Ast
    pub fn parse(source: &'source str) -> Result<Ast, ParserError> {
        let capacity_guess = source.len() / 4;
        let mut parser = Parser {
            ast: Ast::with_capacity(capacity_guess),
            constants: ConstantPoolBuilder::default(),
            lexer: Lexer::new(source),
            frame_stack: Vec::new(),
        };

        let main_block = parser.parse_main_block()?;
        parser.ast.set_entry_point(main_block);
        parser.ast.set_constants(parser.constants.build());

        Ok(parser.ast)
    }

    // Parses the main 'top-level' block
    fn parse_main_block(&mut self) -> Result<AstIndex, ParserError> {
        self.frame_stack.push(Frame::default());

        let start_span = self.current_span();

        let mut context = ExpressionContext::permissive();
        context.expected_indentation = Indentation::Equal(0);

        let mut body = Vec::new();
        while self.peek_token_with_context(&context).is_some() {
            self.consume_until_token_with_context(&context);

            let Some(expression) = self.parse_line(&ExpressionContext::permissive())? else {
                return self.consume_token_and_error(SyntaxError::ExpectedExpression);
            };

            body.push(expression);

            match self.peek_next_token_on_same_line() {
                Some(Token::NewLine | Token::NewLineIndented) => continue,
                None => break,
                _ => return self.consume_token_and_error(SyntaxError::UnexpectedToken),
            }
        }

        // Check that all tokens were consumed
        self.consume_until_token_with_context(&ExpressionContext::permissive());
        if self.peek_token().is_some() {
            return self.consume_token_and_error(SyntaxError::UnexpectedToken);
        }

        let result = self.push_node_with_start_span(
            Node::MainBlock {
                body,
                local_count: self.frame()?.local_count(),
            },
            start_span,
        )?;

        self.frame_stack.pop();
        Ok(result)
    }

    // Attempts to parse an indented block after the current positon
    //
    // e.g.
    //   my_function = |x, y| # <- Here at entry
    //     x = y + 1          # | < indented block
    //     foo x              # | < indented block
    fn parse_indented_block(&mut self) -> Result<Option<AstIndex>, ParserError> {
        let block_context = ExpressionContext::permissive();

        let start_indent = self.current_indent();
        match self.peek_token_with_context(&block_context) {
            Some(peeked) if peeked.indent > start_indent => {}
            _ => return Ok(None), // No indented block found
        }

        let block_context = self
            .consume_until_token_with_context(&block_context)
            .unwrap(); // Safe to unwrap here given that we've just peeked
        let start_span = self.current_span();

        let mut block = Vec::new();
        loop {
            let line_context = ExpressionContext {
                allow_map_block: block.is_empty(),
                ..ExpressionContext::permissive()
            };

            let Some(expression) = self.parse_line(&line_context)? else {
                break;
            };

            block.push(expression);

            match self.peek_next_token_on_same_line() {
                None => break,
                Some(Token::NewLine | Token::NewLineIndented) => {}
                _ => return self.consume_token_and_error(SyntaxError::UnexpectedToken),
            }

            // Peek ahead to see if the indented block continues after this line
            if self.peek_token_with_context(&block_context).is_none() {
                break;
            }

            self.consume_until_token_with_context(&block_context);
        }

        // If the block is a single expression then it doesn't need to be wrapped in a Block node
        if block.len() == 1 {
            Ok(Some(*block.first().unwrap()))
        } else {
            self.push_node_with_start_span(Node::Block(block), start_span)
                .map(Some)
        }
    }

    // Parses expressions from the start of a line
    fn parse_line(&mut self, context: &ExpressionContext) -> Result<Option<AstIndex>, ParserError> {
        self.parse_expressions(context, TempResult::No)
    }

    // Parse a comma separated series of expressions
    //
    // If only a single expression is encountered then that expression's node is the result.
    //
    // Otherwise, for multiple expressions, the result of the expression can be temporary
    // (i.e. not assigned to an identifier) in which case a TempTuple is generated,
    // otherwise the result will be a Tuple.
    fn parse_expressions(
        &mut self,
        context: &ExpressionContext,
        temp_result: TempResult,
    ) -> Result<Option<AstIndex>, ParserError> {
        let start_line = self.current_line_number();

        let mut expression_context = ExpressionContext {
            allow_space_separated_call: true,
            ..*context
        };

        let Some(first) = self.parse_expression(&expression_context)? else {
            return Ok(None);
        };

        let mut expressions = vec![first];
        let mut encountered_linebreak = false;
        let mut encountered_comma = false;

        while let Some(Token::Comma) = self.peek_next_token_on_same_line() {
            self.consume_next_token_on_same_line();

            encountered_comma = true;

            if !encountered_linebreak && self.current_line_number() > start_line {
                // e.g.
                //   x, y =
                //     1, # <- We're here, and want following values to have matching
                //        #    indentation
                //     0
                expression_context = expression_context
                    .with_expected_indentation(Indentation::Equal(self.current_indent()));
                encountered_linebreak = true;
            }

            //
            if let Some(next_expression) =
                self.parse_expression_start(&expressions, 0, &expression_context)?
            {
                match self.ast.node(next_expression).node {
                    Node::Assign { .. }
                    | Node::MultiAssign { .. }
                    | Node::For(_)
                    | Node::While { .. }
                    | Node::Until { .. } => {
                        // These nodes will have consumed the parsed expressions,
                        // so there's no further work to do.
                        // e.g.
                        //   x, y for x, y in a, b
                        //   a, b = c, d
                        //   a, b, c = x
                        return Ok(Some(next_expression));
                    }
                    _ => {}
                }

                expressions.push(next_expression);
            }
        }

        self.frame_mut()?.finalize_id_accesses();

        if expressions.len() == 1 && !encountered_comma {
            Ok(Some(first))
        } else {
            let result = match temp_result {
                TempResult::No => Node::Tuple(expressions),
                TempResult::Yes => Node::TempTuple(expressions),
            };
            Ok(Some(self.push_node(result)?))
        }
    }

    // Parses a single expression
    //
    // Unlike parse_expressions() (which will consume a comma-separated series of expressions),
    // parse_expression() will stop when a comma is encountered.
    fn parse_expression(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<Option<AstIndex>, ParserError> {
        self.parse_expression_with_min_precedence(0, context)
    }

    // Parses a single expression with a specified minimum operator precedence
    fn parse_expression_with_min_precedence(
        &mut self,
        min_precedence: u8,
        context: &ExpressionContext,
    ) -> Result<Option<AstIndex>, ParserError> {
        let result = self.parse_expression_start(&[], min_precedence, context)?;

        match self.peek_next_token_on_same_line() {
            Some(Token::Range | Token::RangeInclusive) => self.parse_range(result, context),
            _ => Ok(result),
        }
    }

    // Parses a term, and then checks to see if the expression is continued
    //
    // When parsing comma-separated expressions, the previous expressions are passed in so that
    // if an assignment operator is encountered then the overall expression is treated as a
    // multi-assignment.
    fn parse_expression_start(
        &mut self,
        previous_expressions: &[AstIndex],
        min_precedence: u8,
        context: &ExpressionContext,
    ) -> Result<Option<AstIndex>, ParserError> {
        let entry_indent = self.current_indent();
        let entry_line = self.current_line_number();

        // Look ahead to get the indent of the first term in the expression.
        // We need to look ahead here because the term may contain its own indentation,
        // so it may end with different indentation.
        let expression_start_info = self.peek_token_with_context(context);

        let expression_start = match self.parse_term(context)? {
            Some(term) => term,
            None => return Ok(None),
        };

        // Safety: it's OK to unwrap here given that a term was successfully parsed
        let expression_start_info = expression_start_info.unwrap();

        let continuation_context = if self.current_line_number() > entry_line {
            if expression_start_info.line == entry_line {
                // The term started on the entry line and ended on a following line.
                //
                // e.g.
                //   foo = ( 1
                //           + 2 )
                //         + 3
                // # ^ entry indent
                // # ^ expression start indent
                // #         ^ expression end indent
                // #       ^ continuation indent
                //
                // A continuation of the expression from here should then be greater than the entry
                // indent, rather than greater than the current (expression end) indent.
                context.with_expected_indentation(Indentation::GreaterThan(entry_indent))
            } else {
                // The term started on a following line.
                //
                // An indent has already occurred for the start term, so then we can allow an
                // expression to continue with greater or equal indentation.
                //
                // e.g.
                //   foo =
                //     ( 1
                //       + 2 )
                //     + 3
                // # ^ entry indent
                // #   ^ expression start indent
                // #     ^ expression end indent
                // #   ^ continuation indent
                //
                // A continuation of the expression from here should be allowed to match the
                // expression start indent.
                context.with_expected_indentation(Indentation::GreaterOrEqual(
                    expression_start_info.indent,
                ))
            }
        } else {
            *context
        };

        self.parse_expression_continued(
            expression_start,
            previous_expressions,
            min_precedence,
            &continuation_context,
        )
    }

    // Parses the continuation of an expression_context
    //
    // Checks for an operator, and then parses the following expressions as the RHS of a binary
    // operation.
    fn parse_expression_continued(
        &mut self,
        expression_start: AstIndex,
        previous_expressions: &[AstIndex],
        min_precedence: u8,
        context: &ExpressionContext,
    ) -> Result<Option<AstIndex>, ParserError> {
        let context = match context.expected_indentation {
            Indentation::Equal(indent) => {
                // If the context has fixed indentation (e.g. at the start of an indented block),
                // allow the indentation to increase
                context.with_expected_indentation(Indentation::GreaterOrEqual(indent))
            }
            Indentation::Flexible => {
                // Indentation within an arithmetic expression shouldn't be able to continue with
                // decreased indentation
                context
                    .with_expected_indentation(Indentation::GreaterOrEqual(self.current_indent()))
            }
            _ => *context,
        };

        if let Some(assignment_expression) =
            self.parse_assign_expression(expression_start, previous_expressions, &context)?
        {
            return Ok(Some(assignment_expression));
        } else if let Some(next) = self.peek_token_with_context(&context) {
            if let Some((left_priority, right_priority)) = operator_precedence(next.token) {
                if left_priority >= min_precedence {
                    let (op, context) = self.consume_token_with_context(&context).unwrap();
                    let op_span = self.current_span();

                    // Move on to the token after the operator
                    if self.peek_token_with_context(&context).is_none() {
                        return self.consume_token_on_same_line_and_error(
                            ExpectedIndentation::RhsExpression,
                        );
                    }
                    let context = self.consume_until_token_with_context(&context).unwrap();

                    let Some(rhs) = self.parse_expression_start(&[], right_priority, &context)?
                    else {
                        return self.consume_token_on_same_line_and_error(
                            ExpectedIndentation::RhsExpression,
                        );
                    };

                    use Token::*;
                    let ast_op = match op {
                        Add => AstBinaryOp::Add,
                        Subtract => AstBinaryOp::Subtract,
                        Multiply => AstBinaryOp::Multiply,
                        Divide => AstBinaryOp::Divide,
                        Remainder => AstBinaryOp::Remainder,

                        AddAssign => AstBinaryOp::AddAssign,
                        SubtractAssign => AstBinaryOp::SubtractAssign,
                        MultiplyAssign => AstBinaryOp::MultiplyAssign,
                        DivideAssign => AstBinaryOp::DivideAssign,
                        RemainderAssign => AstBinaryOp::RemainderAssign,

                        Equal => AstBinaryOp::Equal,
                        NotEqual => AstBinaryOp::NotEqual,

                        Greater => AstBinaryOp::Greater,
                        GreaterOrEqual => AstBinaryOp::GreaterOrEqual,
                        Less => AstBinaryOp::Less,
                        LessOrEqual => AstBinaryOp::LessOrEqual,

                        And => AstBinaryOp::And,
                        Or => AstBinaryOp::Or,

                        Pipe => AstBinaryOp::Pipe,

                        _ => unreachable!(), // The list of tokens here matches the operators in
                                             // operator_precedence()
                    };

                    let op_node = self.push_node_with_span(
                        Node::BinaryOp {
                            op: ast_op,
                            lhs: expression_start,
                            rhs,
                        },
                        op_span,
                    )?;

                    return self.parse_expression_continued(op_node, &[], min_precedence, &context);
                }
            }
        }

        Ok(Some(expression_start))
    }

    // Parses an assignment expression
    //
    // In a multi-assignment expression the LHS can be a series of targets. The last target in the
    // series will be passed in as `lhs`, with the previous targets passed in as `previous_lhs`.
    //
    // If the assignment is an export then operators other than `=` will be rejected.
    fn parse_assign_expression(
        &mut self,
        lhs: AstIndex,
        previous_lhs: &[AstIndex],
        context: &ExpressionContext,
    ) -> Result<Option<AstIndex>, ParserError> {
        match self
            .peek_token_with_context(context)
            .map(|token| token.token)
        {
            Some(Token::Assign) => {}
            _ => return Ok(None),
        }

        let mut targets = Vec::with_capacity(previous_lhs.len() + 1);

        for lhs_expression in previous_lhs.iter().chain(std::iter::once(&lhs)) {
            // Note which identifiers are being assigned to
            match self.ast.node(*lhs_expression).node.clone() {
                Node::Id(id_index) => {
                    self.frame_mut()?.add_local_id_assignment(id_index);
                }
                Node::Meta { .. } | Node::Lookup(_) | Node::Wildcard(_) => {}
                _ => return self.error(SyntaxError::ExpectedAssignmentTarget),
            }

            targets.push(*lhs_expression);
        }

        if targets.is_empty() {
            return self.error(InternalError::MissingAssignmentTarget);
        }

        // Consume the `=` token
        self.consume_token_with_context(context);
        let assign_span = self.current_span();

        let single_target = targets.len() == 1;

        let temp_result = if single_target {
            TempResult::No
        } else {
            TempResult::Yes
        };

        if let Some(rhs) = self.parse_expressions(&ExpressionContext::permissive(), temp_result)? {
            let node = if single_target {
                Node::Assign {
                    target: *targets.first().unwrap(),
                    expression: rhs,
                }
            } else {
                Node::MultiAssign {
                    targets,
                    expression: rhs,
                }
            };
            Ok(Some(self.push_node_with_span(node, assign_span)?))
        } else {
            self.consume_token_on_same_line_and_error(ExpectedIndentation::AssignmentExpression)
        }
    }

    // Peeks the next token and dispatches to the relevant parsing functions
    fn parse_term(&mut self, context: &ExpressionContext) -> Result<Option<AstIndex>, ParserError> {
        use Node::*;

        let start_span = self.current_span();
        let start_indent = self.current_indent();

        let Some(peeked) = self.peek_token_with_context(context) else {
            return Ok(None);
        };

        let result = match peeked.token {
            Token::Null => {
                self.consume_token_with_context(context);
                self.push_node(Null)
            }
            Token::True => {
                self.consume_token_with_context(context);
                self.push_node(BoolTrue)
            }
            Token::False => {
                self.consume_token_with_context(context);
                self.push_node(BoolFalse)
            }
            Token::RoundOpen => self.parse_tuple(context),
            Token::Number => self.parse_number(false, context),
            Token::DoubleQuote | Token::SingleQuote => {
                let (string, span, string_context) = self.parse_string(context)?.unwrap();

                if self.peek_token() == Some(Token::Colon) {
                    self.parse_braceless_map_start(MapKey::Str(string), start_span, &string_context)
                } else {
                    let string_node = self.push_node_with_span(Str(string), span)?;
                    self.check_for_lookup_after_node(string_node, &string_context)
                }
            }
            Token::Id => self.parse_id_expression(context),
            Token::Self_ => self.parse_self_expression(context),
            Token::At => {
                let map_block_allowed = context.allow_map_block || peeked.indent > start_indent;

                let meta_context = self.consume_until_token_with_context(context).unwrap();
                // Safe to unwrap here, parse_meta_key would error on invalid key
                let (meta_key_id, meta_name) = self.parse_meta_key()?.unwrap();

                if map_block_allowed
                    && matches!(
                        self.peek_token_with_context(context),
                        Some(PeekInfo {
                            token: Token::Colon,
                            ..
                        })
                    )
                {
                    self.parse_braceless_map_start(
                        MapKey::Meta(meta_key_id, meta_name),
                        start_span,
                        &meta_context,
                    )
                } else {
                    let meta_key = self.push_node(Node::Meta(meta_key_id, meta_name))?;
                    match self.parse_assign_expression(meta_key, &[], &meta_context)? {
                        Some(result) => self.push_node(Node::Export(result)),
                        None => self
                            .consume_token_and_error(SyntaxError::ExpectedAssignmentAfterMetaKey),
                    }
                }
            }
            Token::Wildcard => self.parse_wildcard(context),
            Token::SquareOpen => self.parse_list(context),
            Token::CurlyOpen => self.parse_map_with_braces(context),
            Token::If => self.parse_if_expression(context),
            Token::Match => self.parse_match_expression(context),
            Token::Switch => self.parse_switch_expression(context),
            Token::Function => self.parse_function(context),
            Token::Subtract => match self.peek_token_n(peeked.peek_count + 1) {
                Some(token) if token.is_whitespace() || token.is_newline() => return Ok(None),
                Some(Token::Number) => {
                    self.consume_token_with_context(context); // Token::Subtract
                    self.parse_number(true, context)
                }
                Some(_) => {
                    self.consume_token_with_context(context); // Token::Subtract
                    if let Some(term) = self.parse_term(&ExpressionContext::restricted())? {
                        self.push_node(Node::UnaryOp {
                            op: AstUnaryOp::Negate,
                            value: term,
                        })
                    } else {
                        self.consume_token_and_error(SyntaxError::ExpectedExpression)
                    }
                }
                None => return Ok(None),
            },
            Token::Not => {
                self.consume_token_with_context(context);
                if let Some(expression) = self.parse_expression(&ExpressionContext {
                    allow_space_separated_call: true,
                    expected_indentation: Indentation::Greater,
                    ..*context
                })? {
                    self.push_node(Node::UnaryOp {
                        op: AstUnaryOp::Not,
                        value: expression,
                    })
                } else {
                    self.consume_token_and_error(SyntaxError::ExpectedExpression)
                }
            }
            Token::Yield => {
                self.consume_token_with_context(context);
                if let Some(expression) =
                    self.parse_expressions(&context.start_new_expression(), TempResult::No)?
                {
                    self.frame_mut()?.contains_yield = true;
                    self.push_node(Node::Yield(expression))
                } else {
                    self.consume_token_and_error(SyntaxError::ExpectedExpression)
                }
            }
            Token::Loop => self.parse_loop_block(context),
            Token::For => self.parse_for_loop(context),
            Token::While => self.parse_while_loop(context),
            Token::Until => self.parse_until_loop(context),
            Token::Break => {
                self.consume_token_with_context(context);
                let break_value =
                    self.parse_expressions(&context.start_new_expression(), TempResult::No)?;
                self.push_node(Node::Break(break_value))
            }
            Token::Continue => {
                self.consume_token_with_context(context);
                self.push_node(Node::Continue)
            }
            Token::Return => {
                self.consume_token_with_context(context);
                let return_value =
                    self.parse_expressions(&context.start_new_expression(), TempResult::No)?;
                self.push_node(Node::Return(return_value))
            }
            Token::Throw => self.parse_throw_expression(),
            Token::Debug => self.parse_debug_expression(),
            Token::From | Token::Import => self.parse_import(context),
            Token::Export => self.parse_export(context),
            Token::Try => self.parse_try_expression(context),
            Token::Error => self.consume_token_and_error(SyntaxError::LexerError),
            _ => return Ok(None),
        };

        result.map(Some)
    }

    // Parses a function
    //
    // e.g.
    //   f = |x, y| x + y
    //   #   ^ You are here
    fn parse_function(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        let start_indent = self.current_indent();

        self.consume_token_with_context(context); // Token::Function

        let span_start = self.current_span().start;

        // Parse function's args
        let mut arg_nodes = Vec::new();
        let mut arg_ids = Vec::new();
        let mut is_variadic = false;

        let mut args_context = ExpressionContext::permissive();
        while self.peek_token_with_context(&args_context).is_some() {
            args_context = self
                .consume_until_token_with_context(&args_context)
                .unwrap();
            match self.parse_id_or_wildcard(context)? {
                Some(IdOrWildcard::Id(constant_index)) => {
                    arg_ids.push(constant_index);
                    arg_nodes.push(self.push_node(Node::Id(constant_index))?);

                    if self.peek_token() == Some(Token::Ellipsis) {
                        self.consume_token();
                        is_variadic = true;
                        break;
                    }
                }
                Some(IdOrWildcard::Wildcard(maybe_id)) => {
                    arg_nodes.push(self.push_node(Node::Wildcard(maybe_id))?)
                }
                None => {
                    match self.peek_token() {
                        Some(Token::Self_) => {
                            self.consume_token();
                            return self.error(SyntaxError::SelfArg);
                        }
                        Some(Token::SquareOpen) => {
                            self.consume_token();
                            let nested_span_start = self.current_span();

                            let list_args = self.parse_nested_function_args(&mut arg_ids)?;
                            if !matches!(
                                self.consume_token_with_context(&args_context),
                                Some((Token::SquareClose, _))
                            ) {
                                return self.error(SyntaxError::ExpectedListEnd);
                            }
                            arg_nodes.push(self.push_node_with_start_span(
                                Node::List(list_args),
                                nested_span_start,
                            )?);
                        }
                        Some(Token::RoundOpen) => {
                            self.consume_token();
                            let nested_span_start = self.current_span();

                            let tuple_args = self.parse_nested_function_args(&mut arg_ids)?;
                            if !matches!(
                                self.consume_token_with_context(&args_context),
                                Some((Token::RoundClose, _))
                            ) {
                                return self.error(SyntaxError::ExpectedCloseParen);
                            }
                            arg_nodes.push(self.push_node_with_start_span(
                                Node::Tuple(tuple_args),
                                nested_span_start,
                            )?);
                        }
                        _ => break,
                    }
                }
            }

            if self.peek_next_token_on_same_line() == Some(Token::Comma) {
                self.consume_next_token_on_same_line();
            } else {
                break;
            }
        }

        // Check for function args end
        let function_end_context = ExpressionContext::permissive()
            .with_expected_indentation(Indentation::Equal(start_indent));
        if !matches!(
            self.consume_token_with_context(&function_end_context),
            Some((Token::Function, _))
        ) {
            return self.error(SyntaxError::ExpectedFunctionArgsEnd);
        }

        // body
        let mut function_frame = Frame::default();
        function_frame.ids_assigned_in_frame.extend(arg_ids.iter());
        self.frame_stack.push(function_frame);

        let body = if let Some(block) = self.parse_indented_block()? {
            block
        } else {
            self.consume_until_next_token_on_same_line();
            if let Some(body) = self.parse_line(&ExpressionContext::permissive())? {
                body
            } else {
                return self.consume_token_and_error(ExpectedIndentation::FunctionBody);
            }
        };

        let function_frame = self
            .frame_stack
            .pop()
            .ok_or_else(|| self.make_error(InternalError::MissingFrame))?;

        self.frame_mut()?
            .add_nested_accessed_non_locals(&function_frame);

        let local_count = function_frame.local_count();

        let span_end = self.current_span().end;

        self.ast.push(
            Node::Function(Function {
                args: arg_nodes,
                local_count,
                accessed_non_locals: Vec::from_iter(function_frame.accessed_non_locals),
                body,
                is_variadic,
                is_generator: function_frame.contains_yield,
            }),
            Span {
                start: span_start,
                end: span_end,
            },
        )
    }

    // Helper for parse_function() that recursively parses nested function arguments
    // e.g.
    //   f = |(foo, bar, [x, y])|
    //   #     ^ You are here
    //   #                ^ ...or here
    fn parse_nested_function_args(
        &mut self,
        arg_ids: &mut Vec<u32>,
    ) -> Result<Vec<AstIndex>, ParserError> {
        let mut nested_args = Vec::new();

        let args_context = ExpressionContext::permissive();
        while self.peek_token_with_context(&args_context).is_some() {
            self.consume_until_token_with_context(&args_context);
            match self.parse_id_or_wildcard(&args_context)? {
                Some(IdOrWildcard::Id(constant_index)) => {
                    if self.constants.get_str(constant_index) == "self" {
                        return self.error(SyntaxError::SelfArg);
                    }

                    let arg_node = if self.peek_token() == Some(Token::Ellipsis) {
                        self.consume_token();
                        Node::Ellipsis(Some(constant_index))
                    } else {
                        Node::Id(constant_index)
                    };

                    nested_args.push(self.push_node(arg_node)?);
                    arg_ids.push(constant_index);
                }
                Some(IdOrWildcard::Wildcard(maybe_id)) => {
                    nested_args.push(self.push_node(Node::Wildcard(maybe_id))?)
                }
                None => match self.peek_token() {
                    Some(Token::SquareOpen) => {
                        self.consume_token();
                        let span_start = self.current_span();

                        let list_args = self.parse_nested_function_args(arg_ids)?;
                        if !matches!(
                            self.consume_token_with_context(&args_context),
                            Some((Token::SquareClose, _))
                        ) {
                            return self.error(SyntaxError::ExpectedListEnd);
                        }
                        nested_args.push(
                            self.push_node_with_start_span(Node::List(list_args), span_start)?,
                        );
                    }
                    Some(Token::RoundOpen) => {
                        self.consume_token();
                        let span_start = self.current_span();

                        let tuple_args = self.parse_nested_function_args(arg_ids)?;
                        if !matches!(
                            self.consume_token_with_context(&args_context),
                            Some((Token::RoundClose, _))
                        ) {
                            return self.error(SyntaxError::ExpectedCloseParen);
                        }
                        nested_args.push(
                            self.push_node_with_start_span(Node::Tuple(tuple_args), span_start)?,
                        );
                    }
                    Some(Token::Ellipsis) => {
                        self.consume_token();
                        nested_args.push(self.push_node(Node::Ellipsis(None))?);
                    }
                    _ => break,
                },
            }

            if self.peek_next_token_on_same_line() == Some(Token::Comma) {
                self.consume_next_token_on_same_line();
            } else {
                break;
            }
        }

        Ok(nested_args)
    }

    // Attempts to parse whitespace-separated call args
    //
    // The context is used to determine what kind of argument separation is allowed.
    //
    // The resulting Vec will be empty if no arguments were encountered.
    //
    // See also parse_parenthesized_args.
    fn parse_call_args(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<Vec<AstIndex>, ParserError> {
        let mut args = Vec::new();

        if context.allow_space_separated_call {
            let mut arg_context = ExpressionContext {
                expected_indentation: Indentation::Greater,
                ..*context
            };

            let mut last_arg_line = self.current_line_number();

            while let Some(peeked) = self.peek_token_with_context(&arg_context) {
                let new_line = peeked.line > last_arg_line;
                last_arg_line = peeked.line;

                if new_line {
                    arg_context.expected_indentation = Indentation::Equal(peeked.indent);
                } else if self.peek_token() != Some(Token::Whitespace) {
                    break;
                }

                if let Some(expression) = self
                    .parse_expression_with_min_precedence(MIN_PRECEDENCE_AFTER_PIPE, &arg_context)?
                {
                    args.push(expression);
                } else {
                    break;
                }

                if self.peek_next_token_on_same_line() == Some(Token::Comma) {
                    self.consume_next_token_on_same_line();
                } else {
                    break;
                }
            }
        }

        Ok(args)
    }

    // Parses a single id
    //
    // See also: parse_id_or_wildcard(), parse_id_expression()
    fn parse_id(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<Option<(u32, ExpressionContext)>, ParserError> {
        match self.peek_token_with_context(context) {
            Some(PeekInfo {
                token: Token::Id, ..
            }) => {
                let (_, id_context) = self.consume_token_with_context(context).unwrap();
                let constant_index = self.add_string_constant(self.lexer.slice())?;
                Ok(Some((constant_index, id_context)))
            }
            _ => Ok(None),
        }
    }

    // Parses a single `_` wildcard, along with its optional following id
    fn parse_wildcard(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        self.consume_token_with_context(context);
        let slice = self.lexer.slice();
        let maybe_id = if slice.len() > 1 {
            Some(self.add_string_constant(&slice[1..])?)
        } else {
            None
        };
        self.push_node(Node::Wildcard(maybe_id))
    }

    // Parses either an id or a wildcard
    //
    // Used in function arguments, match expressions, etc.
    fn parse_id_or_wildcard(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<Option<IdOrWildcard>, ParserError> {
        match self.peek_token_with_context(context) {
            Some(PeekInfo {
                token: Token::Id, ..
            }) => {
                self.consume_token_with_context(context);
                self.add_string_constant(self.lexer.slice())
                    .map(|result| Some(IdOrWildcard::Id(result)))
            }
            Some(PeekInfo {
                token: Token::Wildcard,
                ..
            }) => {
                self.consume_token_with_context(context);
                let slice = self.lexer.slice();
                let maybe_id = if slice.len() > 1 {
                    Some(self.add_string_constant(&slice[1..])?)
                } else {
                    None
                };
                Ok(Some(IdOrWildcard::Wildcard(maybe_id)))
            }
            _ => Ok(None),
        }
    }

    fn parse_id_expression(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        let start_span = self.current_span();
        let Some((constant_index, id_context)) = self.parse_id(context)? else {
            return self.consume_token_and_error(InternalError::UnexpectedToken);
        };

        if self.peek_token() == Some(Token::Colon) {
            self.parse_braceless_map_start(MapKey::Id(constant_index), start_span, &id_context)
        } else {
            self.frame_mut()?.add_id_access(constant_index);

            let lookup_context = id_context.lookup_start();
            if self.next_token_is_lookup_start(&lookup_context) {
                let id_index = self.push_node(Node::Id(constant_index))?;
                self.parse_lookup(id_index, &lookup_context)
            } else {
                let start_span = self.current_span();
                let args = self.parse_call_args(&id_context)?;

                if args.is_empty() {
                    self.push_node(Node::Id(constant_index))
                } else {
                    self.push_node_with_start_span(
                        Node::NamedCall {
                            id: constant_index,
                            args,
                        },
                        start_span,
                    )
                }
            }
        }
    }

    fn parse_self_expression(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        let Some((_, self_context)) = self.consume_token_with_context(context) else {
            return self.error(SyntaxError::ExpectedCloseParen);
        };

        let lookup_context = self_context.lookup_start();
        let self_index = self.push_node(Node::Self_)?;

        if self.next_token_is_lookup_start(&lookup_context) {
            self.parse_lookup(self_index, &lookup_context)
        } else {
            Ok(self_index)
        }
    }

    // Checks to see if a lookup starts after the parsed node,
    // and either returns the node if there's no lookup,
    // or uses the node as the start of the lookup.
    fn check_for_lookup_after_node(
        &mut self,
        node: AstIndex,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        let lookup_context = context.lookup_start();
        if self.next_token_is_lookup_start(&lookup_context) {
            self.parse_lookup(node, &lookup_context)
        } else {
            Ok(node)
        }
    }

    // Returns true if the following token is the start of a lookup expression
    //
    // If the following token is on the same line, then it must be the _next_ token,
    // otherwise the context is used to find an indented token on a following line.
    fn next_token_is_lookup_start(&mut self, context: &ExpressionContext) -> bool {
        use Token::*;

        if matches!(self.peek_token(), Some(Dot | SquareOpen | RoundOpen)) {
            true
        } else if context.allow_linebreaks {
            matches!(
                self.peek_token_with_context(context),
                Some(peeked) if peeked.token == Dot
            )
        } else {
            false
        }
    }

    // Parses a lookup expression
    //
    // Lookup expressions are the name used for a chain of map lookups, index operations,
    // and function calls.
    //
    // The root of the lookup (i.e. the initial expression that is followed by `.`, `[`, or `(`)
    // has already been parsed and is passed in as the `root` argument.
    //
    // e.g.
    //   foo.bar()
    //   #  ^ You are here
    //
    // e.g.
    //   y = x[0][1].foo()
    //   #    ^ You are here
    fn parse_lookup(
        &mut self,
        root: AstIndex,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        let mut lookup = Vec::new();
        let mut lookup_line = self.current_line_number();

        let mut node_context = *context;
        let mut node_start_span = self.current_span();
        let restricted = ExpressionContext::restricted();

        lookup.push((LookupNode::Root(root), node_start_span));

        while let Some(token) = self.peek_token() {
            match token {
                // Function call
                Token::RoundOpen => {
                    self.consume_token();

                    let args = self.parse_parenthesized_args()?;

                    lookup.push((
                        LookupNode::Call {
                            args,
                            with_parens: true,
                        },
                        node_start_span,
                    ));
                }
                // Index
                Token::SquareOpen => {
                    self.consume_token();

                    let index_expression = self.parse_index_expression()?;

                    if let Some(Token::SquareClose) = self.consume_next_token_on_same_line() {
                        lookup.push((LookupNode::Index(index_expression), node_start_span));
                    } else {
                        return self.error(SyntaxError::ExpectedIndexEnd);
                    }
                }
                // Map access
                Token::Dot => {
                    self.consume_token();

                    if !matches!(
                        self.peek_token(),
                        Some(Token::Id | Token::SingleQuote | Token::DoubleQuote)
                    ) {
                        // This check prevents detached dot accesses, e.g. `x. foo`
                        return self.error(SyntaxError::ExpectedMapKey);
                    } else if let Some((id, _)) = self.parse_id(&restricted)? {
                        node_start_span = self.current_span();
                        lookup.push((LookupNode::Id(id), node_start_span));
                    } else if let Some((lookup_string, span, _)) = self.parse_string(&restricted)? {
                        node_start_span = span;
                        lookup.push((LookupNode::Str(lookup_string), span));
                    } else {
                        return self.consume_token_and_error(SyntaxError::ExpectedMapKey);
                    }
                }
                _ => {
                    let Some(peeked) = self.peek_token_with_context(&node_context) else {
                        break;
                    };
                    if peeked.token == Token::Dot {
                        // Indented Dot on the next line?

                        // Consume up until the Dot,
                        // which will be picked up on the next iteration
                        node_context = self
                            .consume_until_token_with_context(&node_context)
                            .unwrap();

                        // Check that the next dot is on an indented line
                        if self.current_line_number() == lookup_line {
                            // TODO Error here?
                            break;
                        }

                        // Starting a new line, so space separated calls are allowed
                        node_context.allow_space_separated_call = true;
                    } else {
                        // Attempt to parse trailing call arguments,
                        // e.g.
                        //   x.foo 42, 99
                        //         ~~~~~~
                        //
                        //   x.foo
                        //     42, 99
                        //     ~~~~~~
                        //
                        //   foo
                        //     .bar 123
                        //          ~~~
                        //     .baz
                        //       x, y
                        //       ~~~~
                        //
                        //   foo.takes_a_map_arg
                        //     bar: 42
                        //     ~~~~~~~

                        // Allow a map block if we're on an indented line
                        node_context.allow_map_block = peeked.line > lookup_line;

                        let args = self.parse_call_args(&node_context)?;

                        // Now that space separated args have been parsed,
                        // don't allow any more while we're on the same line.
                        node_context.allow_space_separated_call = false;

                        if args.is_empty() {
                            // No arguments found, so we're at the end of the lookup
                            break;
                        } else {
                            lookup.push((
                                LookupNode::Call {
                                    args,
                                    with_parens: false,
                                },
                                node_start_span,
                            ));
                        }
                    }

                    lookup_line = self.current_line_number();
                }
            }
        }

        // Add the lookup nodes to the AST in reverse order:
        // the final AST index will be the lookup root node.
        let mut next_index = None;
        for (node, span) in lookup.iter().rev() {
            next_index =
                Some(self.push_node_with_span(Node::Lookup((node.clone(), next_index)), *span)?);
        }
        next_index.ok_or_else(|| self.make_error(InternalError::LookupParseFailure))
    }

    // Helper for parse_lookup() that parses an index expression
    //
    // e.g.
    //   foo.bar[10..20]
    //   #       ^ You are here
    fn parse_index_expression(&mut self) -> Result<AstIndex, ParserError> {
        let index_context = ExpressionContext::restricted();

        let result = if let Some(index_expression) = self.parse_expression(&index_context)? {
            match self.peek_token() {
                Some(Token::Range) => {
                    self.consume_token();

                    if let Some(end_expression) = self.parse_expression(&index_context)? {
                        self.push_node(Node::Range {
                            start: index_expression,
                            end: end_expression,
                            inclusive: false,
                        })?
                    } else {
                        self.push_node(Node::RangeFrom {
                            start: index_expression,
                        })?
                    }
                }
                Some(Token::RangeInclusive) => {
                    self.consume_token();

                    if let Some(end_expression) = self.parse_expression(&index_context)? {
                        self.push_node(Node::Range {
                            start: index_expression,
                            end: end_expression,
                            inclusive: true,
                        })?
                    } else {
                        self.push_node(Node::RangeFrom {
                            start: index_expression,
                        })?
                    }
                }
                _ => index_expression,
            }
        } else {
            // Look for RangeTo/RangeFull
            // e.g. x[..10], y[..]
            match self.consume_next_token_on_same_line() {
                Some(Token::Range) => {
                    if let Some(end_expression) = self.parse_expression(&index_context)? {
                        self.push_node(Node::RangeTo {
                            end: end_expression,
                            inclusive: false,
                        })?
                    } else {
                        self.push_node(Node::RangeFull)?
                    }
                }
                Some(Token::RangeInclusive) => {
                    if let Some(end_expression) = self.parse_expression(&index_context)? {
                        self.push_node(Node::RangeTo {
                            end: end_expression,
                            inclusive: true,
                        })?
                    } else {
                        self.push_node(Node::RangeFull)?
                    }
                }
                _ => return self.error(SyntaxError::ExpectedIndexExpression),
            }
        };

        Ok(result)
    }

    // Helper for parse_lookup() that parses the args in a chained function call
    //
    // e.g.
    // foo[0].bar(1, 2, 3)
    // #          ^ You are here
    fn parse_parenthesized_args(&mut self) -> Result<Vec<AstIndex>, ParserError> {
        let start_indent = self.current_indent();
        let mut args = Vec::new();
        let mut args_context = ExpressionContext::permissive();

        while self.peek_token_with_context(&args_context).is_some() {
            args_context = self
                .consume_until_token_with_context(&args_context)
                .unwrap();

            if let Some(expression) = self.parse_expression(&ExpressionContext::inline())? {
                args.push(expression);
            } else {
                break;
            }

            if self.peek_next_token_on_same_line() == Some(Token::Comma) {
                self.consume_next_token_on_same_line();
            } else {
                break;
            }
        }

        let mut args_end_context = ExpressionContext::permissive();
        args_end_context.expected_indentation = Indentation::Equal(start_indent);
        if !matches!(
            self.consume_token_with_context(&args_end_context),
            Some((Token::RoundClose, _))
        ) {
            return self.error(SyntaxError::ExpectedArgsEnd);
        }

        Ok(args)
    }

    fn parse_range(
        &mut self,
        lhs: Option<AstIndex>,
        context: &ExpressionContext,
    ) -> Result<Option<AstIndex>, ParserError> {
        use Node::{Range, RangeFrom, RangeFull, RangeTo};

        let mut start_span = self.current_span();

        let inclusive = match self.peek_next_token_on_same_line() {
            Some(Token::Range) => false,
            Some(Token::RangeInclusive) => true,
            _ => return Ok(None),
        };

        self.consume_next_token_on_same_line();

        if lhs.is_none() {
            // e.g.
            // for x in ..10
            //          ^^ <- we want the span to start here if we don't have a LHS
            start_span = self.current_span();
        }

        let rhs = self.parse_expression(&ExpressionContext::inline())?;

        let range_node = match (lhs, rhs) {
            (Some(start), Some(end)) => Range {
                start,
                end,
                inclusive,
            },
            (Some(start), None) => RangeFrom { start },
            (None, Some(end)) => RangeTo { end, inclusive },
            (None, None) => RangeFull,
        };

        let range_node = self.push_node_with_start_span(range_node, start_span)?;
        self.check_for_lookup_after_node(range_node, context)
            .map(Some)
    }

    fn parse_export(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        self.consume_token_with_context(context); // Token::Export

        let start_span = self.current_span();

        let Some(expression) = self.parse_expression(&ExpressionContext::permissive())? else {
            return self.consume_token_and_error(SyntaxError::ExpectedExpression);
        };

        self.push_node_with_start_span(Node::Export(expression), start_span)
    }

    fn parse_throw_expression(&mut self) -> Result<AstIndex, ParserError> {
        self.consume_next_token_on_same_line(); // Token::Throw

        let start_span = self.current_span();

        let Some(expression) = self.parse_expression(&ExpressionContext::permissive())? else {
            return self.consume_token_and_error(SyntaxError::ExpectedExpression);
        };

        self.push_node_with_start_span(Node::Throw(expression), start_span)
    }

    fn parse_debug_expression(&mut self) -> Result<AstIndex, ParserError> {
        self.consume_next_token_on_same_line(); // Token::Debug

        let start_position = self.current_span().start;

        self.consume_until_next_token_on_same_line();

        let context = ExpressionContext::permissive();
        let expression_source_start = self.lexer.source_position();
        let Some(expression) = self.parse_expressions(&context, TempResult::No)? else {
            return self.consume_token_and_error(SyntaxError::ExpectedExpression);
        };

        let expression_source_end = self.lexer.source_position();

        let expression_string = self.add_string_constant(
            &self.lexer.source()[expression_source_start..expression_source_end],
        )?;

        self.ast.push(
            Node::Debug {
                expression_string,
                expression,
            },
            Span {
                start: start_position,
                end: self.current_span().end,
            },
        )
    }

    fn parse_number(
        &mut self,
        negate: bool,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        use Node::*;

        self.consume_token_with_context(context); // Token::Number

        let slice = self.lexer.slice();

        let maybe_integer = if let Some(hex) = slice.strip_prefix("0x") {
            i64::from_str_radix(hex, 16)
        } else if let Some(octal) = slice.strip_prefix("0o") {
            i64::from_str_radix(octal, 8)
        } else if let Some(binary) = slice.strip_prefix("0b") {
            i64::from_str_radix(binary, 2)
        } else {
            i64::from_str(slice)
        };

        let number_node = if let Ok(n) = maybe_integer {
            // Should we store the number as a SmallInt or as a stored constant?
            if u8::try_from(n).is_ok() {
                let n = if negate { -n } else { n };
                self.push_node(SmallInt(n as i16))?
            } else {
                let n = if negate { -n } else { n };
                match self.constants.add_i64(n) {
                    Ok(constant_index) => self.push_node(Int(constant_index))?,
                    Err(_) => return self.error(InternalError::ConstantPoolCapacityOverflow),
                }
            }
        } else {
            match f64::from_str(slice) {
                Ok(n) => {
                    let n = if negate { -n } else { n };
                    match self.constants.add_f64(n) {
                        Ok(constant_index) => self.push_node(Float(constant_index))?,
                        Err(_) => return self.error(InternalError::ConstantPoolCapacityOverflow),
                    }
                }
                Err(_) => {
                    return self.error(InternalError::NumberParseFailure);
                }
            }
        };

        self.check_for_lookup_after_node(number_node, context)
    }

    // Parses expressions contained in round parentheses
    // The result may be:
    //   - Null
    //     - e.g. `()`
    //   - A single expression
    //     - e.g. `(1 + 1)`
    //   - A comma-separated tuple
    //     - e.g. `(,)`, `(x,)`, `(1, 2)`
    fn parse_tuple(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        self.consume_token_with_context(context); // Token::RoundOpen

        let start_span = self.current_span();
        let start_indent = self.current_indent();

        let (entries, last_token_was_a_comma) =
            self.parse_comma_separated_entries(Token::RoundClose)?;

        let expressions_node = match entries.as_slice() {
            [] if !last_token_was_a_comma => self.push_node(Node::Null)?,
            [single_expression] if !last_token_was_a_comma => {
                self.push_node_with_start_span(Node::Nested(*single_expression), start_span)?
            }
            _ => self.push_node_with_start_span(Node::Tuple(entries), start_span)?,
        };

        if let Some((Token::RoundClose, _)) = self.consume_token_with_context(context) {
            self.check_for_lookup_after_node(
                expressions_node,
                &context.with_expected_indentation(Indentation::GreaterThan(start_indent)),
            )
        } else {
            self.error(SyntaxError::ExpectedCloseParen)
        }
    }

    // Parses a list, e.g. `[1, 2, 3]`
    fn parse_list(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        self.consume_token_with_context(context); // Token::SquareOpen

        let start_span = self.current_span();
        let start_indent = self.current_indent();

        let (entries, _) = self.parse_comma_separated_entries(Token::SquareClose)?;

        let list_node = self.push_node_with_start_span(Node::List(entries), start_span)?;

        if let Some((Token::SquareClose, _)) = self.consume_token_with_context(context) {
            self.check_for_lookup_after_node(
                list_node,
                &context.with_expected_indentation(Indentation::GreaterThan(start_indent)),
            )
        } else {
            self.error(SyntaxError::ExpectedListEnd)
        }
    }

    // Helper for parse_list and parse_tuple
    //
    // Returns a Vec of entries along with a bool that's true if the last token before the end
    // was a comma, which is used by parse_tuple to determine how the entries should be
    // parsed.
    fn parse_comma_separated_entries(
        &mut self,
        end_token: Token,
    ) -> Result<(Vec<AstIndex>, bool), ParserError> {
        let mut entries = Vec::new();
        let mut entry_context = ExpressionContext::braced_items_start();
        let mut last_token_was_a_comma = false;

        while matches!(
            self.peek_token_with_context(&entry_context),
            Some(peeked) if peeked.token != end_token)
        {
            self.consume_until_token_with_context(&entry_context);

            if let Some(entry) = self.parse_expression(&entry_context)? {
                entries.push(entry);
                last_token_was_a_comma = false;
            }

            if matches!(
                self.peek_token_with_context(&entry_context),
                Some(PeekInfo {
                    token: Token::Comma,
                    ..
                })
            ) {
                self.consume_token_with_context(&entry_context);

                if last_token_was_a_comma {
                    return self.error(SyntaxError::UnexpectedToken);
                }

                last_token_was_a_comma = true;

                entry_context = ExpressionContext::braced_items_continued();
            } else {
                break;
            }
        }

        Ok((entries, last_token_was_a_comma))
    }

    fn parse_braceless_map_start(
        &mut self,
        first_key: MapKey,
        start_span: Span,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        let start_indent = self.current_indent();

        if !context.allow_map_block {
            return self.error(SyntaxError::ExpectedLineBreakBeforeMapBlock);
        }

        if self.consume_token() != Some(Token::Colon) {
            return self.error(InternalError::ExpectedMapColon);
        }

        if let Some(value) = self.parse_expression(&ExpressionContext::permissive())? {
            let entries = if context.allow_map_block {
                let block_context = ExpressionContext::permissive()
                    .with_expected_indentation(Indentation::Equal(start_indent));
                return self.parse_map_block((first_key, Some(value)), start_span, &block_context);
            } else {
                vec![(first_key, Some(value))]
            };

            self.push_node_with_start_span(Node::Map(entries), start_span)
        } else {
            self.consume_token_and_error(SyntaxError::ExpectedMapValue)
        }
    }

    fn parse_map_block(
        &mut self,
        first_entry: (MapKey, Option<AstIndex>),
        start_span: Span,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        let mut entries = vec![first_entry];

        while self.peek_token_with_context(context).is_some() {
            self.consume_until_token_with_context(context);

            let Some(key) = self.parse_map_key()? else {
                return self.consume_token_and_error(SyntaxError::ExpectedMapEntry);
            };

            if self.peek_next_token_on_same_line() != Some(Token::Colon) {
                return self.consume_token_and_error(SyntaxError::ExpectedMapColon);
            };

            self.consume_next_token_on_same_line();

            if let Some(value) = self.parse_expression(&ExpressionContext::inline())? {
                entries.push((key, Some(value)));
            } else {
                // If a value wasn't found on the same line as the key,
                // look for an indented value
                if let Some(value) = self.parse_indented_block()? {
                    entries.push((key, Some(value)));
                } else {
                    return self.consume_token_and_error(SyntaxError::ExpectedMapValue);
                }
            }
        }

        self.push_node_with_start_span(Node::Map(entries), start_span)
    }

    fn parse_map_with_braces(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        self.consume_token_with_context(context); // Token::CurlyOpen

        let start_indent = self.current_indent();
        let start_span = self.current_span();

        let entries = self.parse_comma_separated_map_entries()?;

        let mut map_end_context = ExpressionContext::permissive();
        map_end_context.expected_indentation = Indentation::Equal(start_indent);
        if !matches!(
            self.consume_token_with_context(&map_end_context),
            Some((Token::CurlyClose, _))
        ) {
            return self.error(SyntaxError::ExpectedMapEnd);
        }

        let map_node = self.push_node_with_start_span(Node::Map(entries), start_span)?;
        self.check_for_lookup_after_node(
            map_node,
            &context.with_expected_indentation(Indentation::GreaterThan(start_indent)),
        )
    }

    fn parse_comma_separated_map_entries(
        &mut self,
    ) -> Result<Vec<(MapKey, Option<AstIndex>)>, ParserError> {
        let mut entries = Vec::new();
        let mut entry_context = ExpressionContext::braced_items_start();

        while self.peek_token_with_context(&entry_context).is_some() {
            self.consume_until_token_with_context(&entry_context);

            let Some(key) = self.parse_map_key()? else {
                break;
            };

            if self.peek_token() == Some(Token::Colon) {
                self.consume_token();

                let value_context = ExpressionContext::permissive();
                if self.peek_token_with_context(&value_context).is_none() {
                    return self.error(SyntaxError::ExpectedMapValue);
                }
                self.consume_until_token_with_context(&value_context);

                if let Some(value) = self.parse_expression(&value_context)? {
                    entries.push((key, Some(value)));
                } else {
                    return self.consume_token_and_error(SyntaxError::ExpectedMapValue);
                }
            } else {
                // valueless map entries are allowed in inline maps,
                // e.g.
                //   bar = -1
                //   x = {foo: 42, bar, baz: 99}
                match key {
                    MapKey::Id(id) => self.frame_mut()?.add_id_access(id),
                    _ => return self.error(SyntaxError::ExpectedMapValue),
                }
                entries.push((key, None));
            }

            if matches!(
                self.peek_token_with_context(&entry_context),
                Some(PeekInfo {
                    token: Token::Comma,
                    ..
                })
            ) {
                self.consume_token_with_context(&entry_context);
                entry_context = ExpressionContext::braced_items_continued();
            } else {
                break;
            }
        }

        Ok(entries)
    }

    // Helper for map parsing, attempts to parse a map key from the current position
    //
    // Map keys come in three flavours, e.g.:
    //   my_map =
    //     regular_id: 1
    //     'string_id': 2
    //     @meta meta_key: 3
    fn parse_map_key(&mut self) -> Result<Option<MapKey>, ParserError> {
        let result = if let Some((id, _)) = self.parse_id(&ExpressionContext::restricted())? {
            Some(MapKey::Id(id))
        } else if let Some((string_key, _span, _string_context)) =
            self.parse_string(&ExpressionContext::restricted())?
        {
            Some(MapKey::Str(string_key))
        } else if let Some((meta_key_id, meta_name)) = self.parse_meta_key()? {
            Some(MapKey::Meta(meta_key_id, meta_name))
        } else {
            None
        };

        Ok(result)
    }

    // Attempts to parse a meta key
    fn parse_meta_key(&mut self) -> Result<Option<(MetaKeyId, Option<u32>)>, ParserError> {
        if self.peek_next_token_on_same_line() != Some(Token::At) {
            return Ok(None);
        }

        self.consume_next_token_on_same_line();

        let mut meta_name = None;

        let meta_key_id = match self.consume_token() {
            Some(Token::Add) => MetaKeyId::Add,
            Some(Token::Subtract) => MetaKeyId::Subtract,
            Some(Token::Multiply) => MetaKeyId::Multiply,
            Some(Token::Divide) => MetaKeyId::Divide,
            Some(Token::Remainder) => MetaKeyId::Remainder,
            Some(Token::AddAssign) => MetaKeyId::AddAssign,
            Some(Token::SubtractAssign) => MetaKeyId::SubtractAssign,
            Some(Token::MultiplyAssign) => MetaKeyId::MultiplyAssign,
            Some(Token::DivideAssign) => MetaKeyId::DivideAssign,
            Some(Token::RemainderAssign) => MetaKeyId::RemainderAssign,
            Some(Token::Less) => MetaKeyId::Less,
            Some(Token::LessOrEqual) => MetaKeyId::LessOrEqual,
            Some(Token::Greater) => MetaKeyId::Greater,
            Some(Token::GreaterOrEqual) => MetaKeyId::GreaterOrEqual,
            Some(Token::Equal) => MetaKeyId::Equal,
            Some(Token::NotEqual) => MetaKeyId::NotEqual,
            Some(Token::Not) => MetaKeyId::Not,
            Some(Token::Id) => match self.lexer.slice() {
                "display" => MetaKeyId::Display,
                "iterator" => MetaKeyId::Iterator,
                "next" => MetaKeyId::Next,
                "next_back" => MetaKeyId::NextBack,
                "negate" => MetaKeyId::Negate,
                "type" => MetaKeyId::Type,
                "base" => MetaKeyId::Base,
                "main" => MetaKeyId::Main,
                "tests" => MetaKeyId::Tests,
                "pre_test" => MetaKeyId::PreTest,
                "post_test" => MetaKeyId::PostTest,
                "test" => match self.consume_next_token_on_same_line() {
                    Some(Token::Id) => {
                        let test_name = self.add_string_constant(self.lexer.slice())?;
                        meta_name = Some(test_name);
                        MetaKeyId::Test
                    }
                    _ => return self.error(SyntaxError::ExpectedTestName),
                },
                "meta" => match self.consume_next_token_on_same_line() {
                    Some(Token::Id) => {
                        let id = self.add_string_constant(self.lexer.slice())?;
                        meta_name = Some(id);
                        MetaKeyId::Named
                    }
                    _ => return self.error(SyntaxError::ExpectedMetaId),
                },
                _ => return self.error(SyntaxError::UnexpectedMetaKey),
            },
            Some(Token::SquareOpen) => match self.consume_token() {
                Some(Token::SquareClose) => MetaKeyId::Index,
                _ => return self.error(SyntaxError::UnexpectedMetaKey),
            },
            Some(Token::Function) => match self.consume_token() {
                Some(Token::Function) => MetaKeyId::Call,
                _ => return self.error(SyntaxError::UnexpectedMetaKey),
            },
            _ => return self.error(SyntaxError::UnexpectedMetaKey),
        };

        Ok(Some((meta_key_id, meta_name)))
    }

    fn parse_for_loop(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        self.consume_token_with_context(context); // Token::For

        let start_span = self.current_span();

        let mut args = Vec::new();
        while let Some(id_or_wildcard) = self.parse_id_or_wildcard(context)? {
            match id_or_wildcard {
                IdOrWildcard::Id(id) => {
                    self.frame_mut()?.ids_assigned_in_frame.insert(id);
                    args.push(self.push_node(Node::Id(id))?);
                }
                IdOrWildcard::Wildcard(maybe_id) => {
                    args.push(self.push_node(Node::Wildcard(maybe_id))?);
                }
            }

            match self.peek_next_token_on_same_line() {
                Some(Token::Comma) => {
                    self.consume_next_token_on_same_line();
                }
                Some(Token::In) => {
                    self.consume_next_token_on_same_line();
                    break;
                }
                _ => return self.consume_token_and_error(SyntaxError::ExpectedForInKeyword),
            }
        }
        if args.is_empty() {
            return self.consume_token_and_error(SyntaxError::ExpectedForArgs);
        }

        let iterable = match self.parse_expression(&ExpressionContext::inline())? {
            Some(iterable) => iterable,
            None => return self.consume_token_and_error(SyntaxError::ExpectedForIterable),
        };

        match self.parse_indented_block()? {
            Some(body) => {
                let result = self.push_node_with_start_span(
                    Node::For(AstFor {
                        args,
                        iterable,
                        body,
                    }),
                    start_span,
                )?;

                Ok(result)
            }
            None => self.consume_token_and_error(ExpectedIndentation::ForBody),
        }
    }

    // Parses a loop declared with the `loop` keyword
    fn parse_loop_block(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        self.consume_token_with_context(context); // Token::Loop

        if let Some(body) = self.parse_indented_block()? {
            self.push_node(Node::Loop { body })
        } else {
            self.consume_token_and_error(ExpectedIndentation::LoopBody)
        }
    }

    fn parse_while_loop(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        self.consume_token_with_context(context); // Token::While

        let Some(condition) = self.parse_expression(&ExpressionContext::inline())? else {
            return self.consume_token_and_error(SyntaxError::ExpectedWhileCondition);
        };

        match self.parse_indented_block()? {
            Some(body) => self.push_node(Node::While { condition, body }),
            None => self.consume_token_and_error(ExpectedIndentation::WhileBody),
        }
    }

    fn parse_until_loop(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        self.consume_token_with_context(context); // Token::Until

        let Some(condition) = self.parse_expression(&ExpressionContext::inline())? else {
            return self.consume_token_and_error(SyntaxError::ExpectedUntilCondition);
        };

        match self.parse_indented_block()? {
            Some(body) => self.push_node(Node::Until { condition, body }),
            None => self.consume_token_and_error(ExpectedIndentation::UntilBody),
        }
    }

    fn parse_if_expression(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        use SyntaxError::*;

        self.consume_token_with_context(context); // Token::If

        let if_span = self.current_span();

        // Define the expected indentation of 'else if' / 'else' blocks
        let mut outer_context =
            context.with_expected_indentation(Indentation::GreaterOrEqual(self.current_indent()));

        let Some(condition) = self.parse_expression(&ExpressionContext::inline())? else {
            return self.consume_token_and_error(ExpectedIfCondition);
        };

        if self.peek_next_token_on_same_line() == Some(Token::Then) {
            self.consume_next_token_on_same_line();
            let Some(then_node) =
                self.parse_expressions(&ExpressionContext::inline(), TempResult::No)?
            else {
                return self.error(ExpectedThenExpression);
            };

            let else_node = if self.peek_next_token_on_same_line() == Some(Token::Else) {
                self.consume_next_token_on_same_line();
                match self.parse_expressions(&ExpressionContext::inline(), TempResult::No)? {
                    Some(else_node) => Some(else_node),
                    None => return self.error(ExpectedElseExpression),
                }
            } else {
                None
            };

            self.push_node_with_span(
                Node::If(AstIf {
                    condition,
                    then_node,
                    else_if_blocks: vec![],
                    else_node,
                }),
                if_span,
            )
        } else {
            if !outer_context.allow_linebreaks {
                return self.error(IfBlockNotAllowedInThisContext);
            }

            if let Some(then_node) = self.parse_indented_block()? {
                let mut else_if_blocks = Vec::new();

                while let Some(peeked) = self.peek_token_with_context(&outer_context) {
                    if peeked.token != Token::ElseIf {
                        break;
                    }

                    self.consume_token_with_context(&outer_context);

                    // Once we've got an else if block, then all following blocks in the
                    // cascade should start with the same indentation.
                    outer_context = context
                        .with_expected_indentation(Indentation::Equal(self.current_indent()));

                    let Some(else_if_condition) =
                        self.parse_expression(&ExpressionContext::inline())?
                    else {
                        return self.consume_token_and_error(ExpectedElseIfCondition);
                    };

                    if let Some(else_if_block) = self.parse_indented_block()? {
                        else_if_blocks.push((else_if_condition, else_if_block));
                    } else {
                        return self.consume_token_on_same_line_and_error(
                            ExpectedIndentation::ElseIfBlock,
                        );
                    }
                }

                let else_node = match self.peek_token_with_context(&outer_context) {
                    Some(peeked) if peeked.token == Token::Else => {
                        self.consume_token_with_context(&outer_context);

                        if let Some(else_block) = self.parse_indented_block()? {
                            Some(else_block)
                        } else {
                            return self.consume_token_on_same_line_and_error(
                                ExpectedIndentation::ElseBlock,
                            );
                        }
                    }
                    _ => None,
                };

                self.push_node_with_span(
                    Node::If(AstIf {
                        condition,
                        then_node,
                        else_if_blocks,
                        else_node,
                    }),
                    if_span,
                )
            } else {
                self.consume_token_on_same_line_and_error(ExpectedIndentation::ThenKeywordOrBlock)
            }
        }
    }

    fn parse_switch_expression(
        &mut self,
        switch_context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        use SyntaxError::*;

        self.consume_token_with_context(switch_context); // Token::Switch

        let current_indent = self.current_indent();
        let switch_span = self.current_span();

        let arm_context = match self.consume_until_token_with_context(switch_context) {
            Some(arm_context) if self.current_indent() > current_indent => arm_context,
            _ => return self.consume_token_on_same_line_and_error(ExpectedIndentation::SwitchArm),
        };

        let mut arms = Vec::new();

        while self.peek_token().is_some() {
            let condition = self.parse_expression(&ExpressionContext::inline())?;

            let arm_body = match self.peek_next_token_on_same_line() {
                Some(Token::Else) => {
                    if condition.is_some() {
                        return self.consume_token_and_error(UnexpectedSwitchElse);
                    }

                    self.consume_next_token_on_same_line();

                    if let Some(expression) =
                        self.parse_expressions(&ExpressionContext::inline(), TempResult::No)?
                    {
                        expression
                    } else if let Some(indented_block) = self.parse_indented_block()? {
                        indented_block
                    } else {
                        return self.consume_token_and_error(ExpectedSwitchArmExpression);
                    }
                }
                Some(Token::Then) => {
                    self.consume_next_token_on_same_line();

                    if let Some(expression) =
                        self.parse_expressions(&ExpressionContext::inline(), TempResult::No)?
                    {
                        expression
                    } else if let Some(indented_block) = self.parse_indented_block()? {
                        indented_block
                    } else {
                        return self.consume_token_and_error(ExpectedSwitchArmExpressionAfterThen);
                    }
                }
                _ => return self.consume_token_and_error(ExpectedSwitchArmExpression),
            };

            arms.push(SwitchArm {
                condition,
                expression: arm_body,
            });

            if self.peek_token_with_context(&arm_context).is_none() {
                break;
            }

            self.consume_until_token_with_context(&arm_context);
        }

        // Check for errors now that the match expression is complete
        for (arm_index, arm) in arms.iter().enumerate() {
            let last_arm = arm_index == arms.len() - 1;

            if arm.condition.is_none() && !last_arm {
                return Err(ParserError::new(SwitchElseNotInLastArm.into(), switch_span));
            }
        }

        self.push_node_with_span(Node::Switch(arms), switch_span)
    }

    fn parse_match_expression(
        &mut self,
        match_context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        use SyntaxError::*;

        self.consume_token_with_context(match_context); // Token::Match

        let current_indent = self.current_indent();
        let match_span = self.current_span();

        let match_expression =
            match self.parse_expressions(&ExpressionContext::inline(), TempResult::Yes)? {
                Some(expression) => expression,
                None => {
                    return self.consume_token_and_error(ExpectedMatchExpression);
                }
            };

        let arm_context = match self.consume_until_token_with_context(match_context) {
            Some(arm_context) if self.current_indent() > current_indent => arm_context,
            _ => return self.consume_token_on_same_line_and_error(ExpectedIndentation::MatchArm),
        };

        let mut arms = Vec::new();

        while self.peek_token().is_some() {
            // Match patterns for a single arm, with alternatives separated by 'or'
            // e.g. match x, y
            //   0, 1 then ...
            //   2, 3 or 4, 5 then ...
            //   other then ...
            let mut arm_patterns = Vec::new();
            let mut expected_arm_count = 1;

            let condition = {
                while let Some(pattern) = self.parse_match_pattern(false)? {
                    // Match patterns, separated by commas in the case of matching multi-expressions
                    let mut patterns = vec![pattern];

                    while let Some(Token::Comma) = self.peek_next_token_on_same_line() {
                        self.consume_next_token_on_same_line();

                        match self.parse_match_pattern(false)? {
                            Some(pattern) => patterns.push(pattern),
                            None => return self.consume_token_and_error(ExpectedMatchPattern),
                        }
                    }

                    arm_patterns.push(match patterns.as_slice() {
                        [single_pattern] => *single_pattern,
                        _ => self.push_node(Node::TempTuple(patterns))?,
                    });

                    if let Some(Token::Or) = self.peek_next_token_on_same_line() {
                        self.consume_next_token_on_same_line();
                        expected_arm_count += 1;
                    }
                }

                if self.peek_next_token_on_same_line() == Some(Token::If) {
                    self.consume_next_token_on_same_line();

                    match self.parse_expression(&ExpressionContext::inline())? {
                        Some(expression) => Some(expression),
                        None => return self.consume_token_and_error(ExpectedMatchCondition),
                    }
                } else {
                    None
                }
            };

            let arm_body = match self.peek_next_token_on_same_line() {
                Some(Token::Else) => {
                    if !arm_patterns.is_empty() || condition.is_some() {
                        return self.consume_token_and_error(UnexpectedMatchElse);
                    }

                    self.consume_next_token_on_same_line();

                    if let Some(expression) =
                        self.parse_expressions(&ExpressionContext::inline(), TempResult::No)?
                    {
                        expression
                    } else if let Some(indented_block) = self.parse_indented_block()? {
                        indented_block
                    } else {
                        return self.consume_token_and_error(ExpectedMatchArmExpression);
                    }
                }
                Some(Token::Then) => {
                    if arm_patterns.len() != expected_arm_count {
                        return self.consume_token_and_error(ExpectedMatchPattern);
                    }

                    self.consume_next_token_on_same_line();

                    if let Some(expression) =
                        self.parse_expressions(&ExpressionContext::inline(), TempResult::No)?
                    {
                        expression
                    } else if let Some(indented_block) = self.parse_indented_block()? {
                        indented_block
                    } else {
                        return self.consume_token_and_error(ExpectedMatchArmExpressionAfterThen);
                    }
                }
                Some(Token::If) => return self.consume_token_and_error(UnexpectedMatchIf),
                _ => return self.consume_token_and_error(ExpectedMatchArmExpression),
            };

            arms.push(MatchArm {
                patterns: arm_patterns,
                condition,
                expression: arm_body,
            });

            if self.peek_token_with_context(&arm_context).is_none() {
                break;
            }

            self.consume_until_token_with_context(&arm_context);
        }

        // Check for errors now that the match expression is complete

        for (arm_index, arm) in arms.iter().enumerate() {
            let last_arm = arm_index == arms.len() - 1;

            if arm.patterns.is_empty() && arm.condition.is_none() && !last_arm {
                return Err(ParserError::new(MatchElseNotInLastArm.into(), match_span));
            }
        }

        self.push_node_with_span(
            Node::Match {
                expression: match_expression,
                arms,
            },
            match_span,
        )
    }

    // Parses a match arm's pattern
    fn parse_match_pattern(
        &mut self,
        in_nested_patterns: bool,
    ) -> Result<Option<AstIndex>, ParserError> {
        use Token::*;

        let pattern_context = ExpressionContext::restricted();

        let result = match self.peek_token_with_context(&pattern_context) {
            Some(peeked) => match peeked.token {
                True | False | Null | Number | SingleQuote | DoubleQuote | Subtract => {
                    return self.parse_term(&pattern_context)
                }
                Id => match self.parse_id(&pattern_context)? {
                    Some((id, _)) => {
                        let result = if self.peek_token() == Some(Ellipsis) {
                            self.consume_token();
                            if in_nested_patterns {
                                self.frame_mut()?.ids_assigned_in_frame.insert(id);
                                self.push_node(Node::Ellipsis(Some(id)))?
                            } else {
                                return self
                                    .error(SyntaxError::MatchEllipsisOutsideOfNestedPatterns);
                            }
                        } else {
                            let id_node = self.push_node(Node::Id(id))?;
                            if self.next_token_is_lookup_start(&pattern_context) {
                                self.frame_mut()?.add_id_access(id);
                                self.parse_lookup(id_node, &pattern_context)?
                            } else {
                                self.frame_mut()?.ids_assigned_in_frame.insert(id);
                                id_node
                            }
                        };
                        Some(result)
                    }
                    None => return self.error(InternalError::IdParseFailure),
                },
                Wildcard => self.parse_wildcard(&pattern_context).map(Some)?,
                SquareOpen => {
                    self.consume_token_with_context(&pattern_context);

                    let list_patterns = self.parse_nested_match_patterns()?;

                    if self.consume_next_token_on_same_line() != Some(SquareClose) {
                        return self.error(SyntaxError::ExpectedListEnd);
                    }

                    Some(self.push_node(Node::List(list_patterns))?)
                }
                RoundOpen => {
                    self.consume_token_with_context(&pattern_context);

                    if self.peek_token() == Some(RoundClose) {
                        self.consume_token();
                        Some(self.push_node(Node::Null)?)
                    } else {
                        let tuple_patterns = self.parse_nested_match_patterns()?;

                        if self.consume_next_token_on_same_line() != Some(RoundClose) {
                            return self.error(SyntaxError::ExpectedCloseParen);
                        }

                        Some(self.push_node(Node::Tuple(tuple_patterns))?)
                    }
                }
                Ellipsis if in_nested_patterns => {
                    self.consume_token_with_context(&pattern_context);
                    Some(self.push_node(Node::Ellipsis(None))?)
                }
                _ => None,
            },
            None => None,
        };

        Ok(result)
    }

    // Recursively parses nested match patterns
    //
    // e.g.
    //   match x
    //     (1, 2, [3, 4]) then ...
    //   #  ^ You are here
    //   #         ^...or here
    fn parse_nested_match_patterns(&mut self) -> Result<Vec<AstIndex>, ParserError> {
        let mut result = vec![];

        while let Some(pattern) = self.parse_match_pattern(true)? {
            result.push(pattern);

            if self.peek_next_token_on_same_line() != Some(Token::Comma) {
                break;
            }
            self.consume_next_token_on_same_line();
        }

        Ok(result)
    }

    fn parse_import(&mut self, context: &ExpressionContext) -> Result<AstIndex, ParserError> {
        let importing_from = match self.peek_token_with_context(context) {
            Some(peeked) if peeked.token == Token::Import => false,
            Some(peeked) if peeked.token == Token::From => true,
            _ => return self.error(InternalError::UnexpectedToken),
        };

        let start_span = self.current_span();
        let from_context = ExpressionContext::restricted();

        self.consume_token_with_context(&from_context);

        let from = if importing_from {
            // Parse the from module path: a nested path is allowed, but only a single path
            let from = match self.parse_import_items(true, &from_context)?.as_slice() {
                [from] => from.clone(),
                _ => return self.error(SyntaxError::ImportFromExpressionHasTooManyItems),
            };

            match self.consume_token_with_context(&from_context) {
                Some((Token::Import, _)) => {}
                _ => return self.error(SyntaxError::ExpectedImportAfterFrom),
            }

            from
        } else {
            vec![]
        };

        // Nested items aren't allowed, flatten the returned items into a single vec
        let items: Vec<ImportItemNode> = self
            .parse_import_items(false, &ExpressionContext::permissive())?
            .into_iter()
            .flatten()
            .collect();

        // Mark any imported ids as locally assigned
        for item in items.iter() {
            match item {
                ImportItemNode::Id(id) => {
                    self.frame_mut()?.ids_assigned_in_frame.insert(*id);
                }
                ImportItemNode::Str(_) => {}
            }
        }

        self.push_node_with_start_span(Node::Import { from, items }, start_span)
    }

    // Helper for parse_import(), parses a series of import items
    // e.g.
    //   from baz.qux import foo, 'bar', 'x'
    //   #    ^ You are here, with nested items allowed
    //   #                   ^ Or here, with nested items disallowed
    fn parse_import_items(
        &mut self,
        allow_nested_items: bool,
        context: &ExpressionContext,
    ) -> Result<Vec<Vec<ImportItemNode>>, ParserError> {
        let mut items = vec![];
        let mut context = *context;

        loop {
            let item_root = match self.parse_id(&context)? {
                Some((id, _)) => ImportItemNode::Id(id),
                None => match self.parse_string(&context)? {
                    Some((import_string, _span, _string_context)) => {
                        ImportItemNode::Str(import_string)
                    }
                    None => break,
                },
            };

            let mut item = vec![item_root];

            if allow_nested_items {
                while self.peek_token() == Some(Token::Dot) {
                    self.consume_token();

                    match self.parse_id(&ExpressionContext::restricted())? {
                        Some((id, _)) => item.push(ImportItemNode::Id(id)),
                        None => match self.parse_string(&ExpressionContext::restricted())? {
                            Some((node_string, _span, _string_context)) => {
                                item.push(ImportItemNode::Str(node_string));
                            }
                            None => {
                                return self
                                    .consume_token_and_error(SyntaxError::ExpectedImportModuleId)
                            }
                        },
                    }
                }
            }

            items.push(item);

            match self.peek_token_with_context(&context) {
                Some(peeked) if peeked.token == Token::Comma => {
                    if let Some((_, new_context)) = self.consume_token_with_context(&context) {
                        context = new_context.with_expected_indentation(
                            Indentation::GreaterOrEqual(self.current_indent()),
                        );
                    }
                }
                Some(peeked) if peeked.token == Token::Dot => {
                    return self.consume_token_and_error(SyntaxError::UnexpectedDotAfterImportItem);
                }
                _ => break,
            }
        }

        if items.is_empty() {
            return self.error(SyntaxError::ExpectedIdInImportExpression);
        }

        Ok(items)
    }

    fn parse_try_expression(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<AstIndex, ParserError> {
        let outer_context = match self.consume_token_with_context(context) {
            Some((Token::Try, outer_context)) => {
                outer_context.with_expected_indentation(Indentation::Equal(self.current_indent()))
            }
            _ => return self.error(InternalError::UnexpectedToken),
        };

        let start_span = self.current_span();

        let Some(try_block) = self.parse_indented_block()? else {
            return self.consume_token_on_same_line_and_error(ExpectedIndentation::TryBody);
        };

        if !matches!(
            self.consume_token_with_context(&outer_context),
            Some((Token::Catch, _))
        ) {
            return self.error(SyntaxError::ExpectedCatch);
        }

        let catch_arg = match self.parse_id_or_wildcard(&ExpressionContext::restricted())? {
            Some(IdOrWildcard::Id(id)) => {
                self.frame_mut()?.ids_assigned_in_frame.insert(id);
                self.push_node(Node::Id(id))?
            }
            Some(IdOrWildcard::Wildcard(maybe_id)) => self.push_node(Node::Wildcard(maybe_id))?,
            None => return self.consume_token_and_error(SyntaxError::ExpectedCatchArgument),
        };

        let Some(catch_block) = self.parse_indented_block()? else {
            return self.consume_token_on_same_line_and_error(ExpectedIndentation::CatchBody);
        };

        let finally_block = match self.peek_token_with_context(&outer_context) {
            Some(peeked) if peeked.token == Token::Finally => {
                self.consume_token_with_context(&outer_context);
                if let Some(finally_block) = self.parse_indented_block()? {
                    Some(finally_block)
                } else {
                    return self
                        .consume_token_on_same_line_and_error(ExpectedIndentation::FinallyBody);
                }
            }
            _ => None,
        };

        self.push_node_with_start_span(
            Node::Try(AstTry {
                try_block,
                catch_arg,
                catch_block,
                finally_block,
            }),
            start_span,
        )
    }

    fn parse_string(
        &mut self,
        context: &ExpressionContext,
    ) -> Result<Option<(AstString, Span, ExpressionContext)>, ParserError> {
        use SyntaxError::*;
        use Token::*;

        match self.peek_token_with_context(context) {
            Some(PeekInfo {
                token: SingleQuote | DoubleQuote,
                ..
            }) => {}
            _ => return Ok(None),
        }

        let (string_quote, string_context) = self.consume_token_with_context(context).unwrap();
        let start_span = self.current_span();
        let mut nodes = Vec::new();

        while let Some(next_token) = self.consume_token() {
            match next_token {
                StringLiteral => {
                    let string_literal = self.lexer.slice();

                    let mut literal = String::with_capacity(string_literal.len());
                    let mut chars = string_literal.chars().peekable();

                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => match chars.next() {
                                Some('\n' | '\r') => {
                                    while let Some(c) = chars.peek() {
                                        if c.is_whitespace() {
                                            chars.next();
                                        } else {
                                            break;
                                        }
                                    }
                                }
                                Some('\\') => literal.push('\\'),
                                Some('\'') => literal.push('\''),
                                Some('$') => literal.push('$'),
                                Some('"') => literal.push('"'),
                                Some('n') => literal.push('\n'),
                                Some('r') => literal.push('\r'),
                                Some('t') => literal.push('\t'),
                                Some('x') => match chars.next() {
                                    Some(c1) if c1.is_ascii_hexdigit() => match chars.next() {
                                        Some(c2) if c2.is_ascii_hexdigit() => {
                                            // is_ascii_hexdigit already checked
                                            let d1 = c1.to_digit(16).unwrap();
                                            let d2 = c2.to_digit(16).unwrap();
                                            let d = d1 * 16 + d2;
                                            if d <= 0x7f {
                                                literal.push(char::from_u32(d).unwrap());
                                            } else {
                                                return self.error(AsciiEscapeCodeOutOfRange);
                                            }
                                        }
                                        Some(_) => {
                                            return self.error(UnexpectedCharInNumericEscapeCode)
                                        }
                                        None => return self.error(UnterminatedNumericEscapeCode),
                                    },
                                    Some(_) => {
                                        return self.error(UnexpectedCharInNumericEscapeCode)
                                    }
                                    None => return self.error(UnterminatedNumericEscapeCode),
                                },
                                Some('u') => match chars.next() {
                                    Some('{') => {
                                        let mut code = 0;

                                        while let Some(c) = chars.peek().cloned() {
                                            if c.is_ascii_hexdigit() {
                                                chars.next();
                                                code *= 16;
                                                code += c.to_digit(16).unwrap();
                                            } else {
                                                break;
                                            }
                                        }

                                        match chars.next() {
                                            Some('}') => match char::from_u32(code) {
                                                Some(c) => literal.push(c),
                                                None => {
                                                    return self.error(UnicodeEscapeCodeOutOfRange);
                                                }
                                            },
                                            Some(_) => {
                                                return self
                                                    .error(UnexpectedCharInNumericEscapeCode);
                                            }
                                            None => {
                                                return self.error(UnterminatedNumericEscapeCode)
                                            }
                                        }
                                    }
                                    Some(_) => {
                                        return self.error(UnexpectedCharInNumericEscapeCode)
                                    }
                                    None => return self.error(UnterminatedNumericEscapeCode),
                                },
                                _ => return self.error(UnexpectedEscapeInString),
                            },
                            _ => literal.push(c),
                        }
                    }

                    nodes.push(StringNode::Literal(self.add_string_constant(&literal)?));
                }
                Dollar => match self.peek_token() {
                    Some(Id) => {
                        self.consume_token();
                        let id = self.add_string_constant(self.lexer.slice())?;
                        self.frame_mut()?.add_id_access(id);
                        let id_node = self.push_node(Node::Id(id))?;
                        nodes.push(StringNode::Expr(id_node));
                    }
                    Some(CurlyOpen) => {
                        self.consume_token();

                        if let Some(expression) =
                            self.parse_expressions(&ExpressionContext::inline(), TempResult::No)?
                        {
                            nodes.push(StringNode::Expr(expression));
                        } else {
                            return self.consume_token_and_error(ExpectedExpression);
                        }

                        if self.consume_token() != Some(CurlyClose) {
                            return self.error(ExpectedStringPlaceholderEnd);
                        }
                    }
                    Some(_) => {
                        return self.consume_token_and_error(UnexpectedTokenAfterDollarInString);
                    }
                    None => break,
                },
                c if c == string_quote => {
                    let quotation_mark = if string_quote == SingleQuote {
                        QuotationMark::Single
                    } else {
                        QuotationMark::Double
                    };

                    if nodes.is_empty() {
                        nodes.push(StringNode::Literal(self.add_string_constant("")?));
                    }

                    return Ok(Some((
                        AstString {
                            quotation_mark,
                            nodes,
                        },
                        self.span_with_start(start_span),
                        string_context,
                    )));
                }
                _ => return self.error(UnexpectedToken),
            }
        }

        self.error(UnterminatedString)
    }

    //// Error helpers

    fn error<E, T>(&mut self, error_type: E) -> Result<T, ParserError>
    where
        E: Into<ParserErrorKind>,
    {
        Err(self.make_error(error_type))
    }

    fn make_error<E>(&mut self, error_type: E) -> ParserError
    where
        E: Into<ParserErrorKind>,
    {
        #[allow(clippy::let_and_return)]
        let error = ParserError::new(error_type.into(), self.current_span());

        #[cfg(feature = "panic_on_parser_error")]
        panic!("{error}");

        error
    }

    fn consume_token_on_same_line_and_error<E, T>(
        &mut self,
        error_type: E,
    ) -> Result<T, ParserError>
    where
        E: Into<ParserErrorKind>,
    {
        self.consume_next_token_on_same_line();
        self.error(error_type)
    }

    fn consume_token_and_error<E, T>(&mut self, error_type: E) -> Result<T, ParserError>
    where
        E: Into<ParserErrorKind>,
    {
        self.consume_token_with_context(&ExpressionContext::permissive());
        self.error(error_type)
    }

    //// Lexer getters

    fn current_line_number(&self) -> u32 {
        self.lexer.line_number()
    }

    fn current_indent(&self) -> usize {
        self.lexer.current_indent()
    }

    fn current_span(&self) -> Span {
        self.lexer.span()
    }

    fn peek_token(&mut self) -> Option<Token> {
        self.lexer.peek()
    }

    fn peek_token_n(&mut self, n: usize) -> Option<Token> {
        self.lexer.peek_n(n)
    }

    fn consume_token(&mut self) -> Option<Token> {
        self.lexer.next()
    }

    //// Node push helpers

    fn push_node(&mut self, node: Node) -> Result<AstIndex, ParserError> {
        self.push_node_with_span(node, self.current_span())
    }

    fn push_node_with_span(&mut self, node: Node, span: Span) -> Result<AstIndex, ParserError> {
        self.ast.push(node, span)
    }

    fn push_node_with_start_span(
        &mut self,
        node: Node,
        start_span: Span,
    ) -> Result<AstIndex, ParserError> {
        self.push_node_with_span(node, self.span_with_start(start_span))
    }

    fn span_with_start(&self, start_span: Span) -> Span {
        Span {
            start: start_span.start,
            end: self.current_span().end,
        }
    }

    fn add_string_constant(&mut self, s: &str) -> Result<u32, ParserError> {
        match self.constants.add_string(s) {
            Ok(result) => Ok(result),
            Err(_) => self.error(InternalError::ConstantPoolCapacityOverflow),
        }
    }

    // Peeks past whitespace, comments, and newlines until the next token is found
    //
    // Tokens on following lines will only be returned if the expression context allows linebreaks.
    //
    // If expected indentation is specified in the expression context, then the next token
    // needs to have matching indentation, otherwise None is returned.
    fn peek_token_with_context(&mut self, context: &ExpressionContext) -> Option<PeekInfo> {
        use Token::*;

        let mut peek_count = 0;
        let start_line = self.current_line_number();
        let start_indent = self.current_indent();

        while let Some(peeked) = self.peek_token_n(peek_count) {
            match peeked {
                Whitespace | NewLine | NewLineIndented | CommentMulti | CommentSingle => {}
                token => {
                    return match self.lexer.peek_line_number(peek_count) {
                        peeked_line if peeked_line == start_line => Some(PeekInfo {
                            token,
                            line: start_line,
                            indent: start_indent,
                            peek_count,
                        }),
                        peeked_line if context.allow_linebreaks => {
                            let peeked_indent = self.lexer.peek_indent(peek_count);
                            let peek_info = PeekInfo {
                                token,
                                line: peeked_line,
                                indent: peeked_indent,
                                peek_count,
                            };

                            use Indentation::*;
                            match context.expected_indentation {
                                GreaterThan(expected_indent) if peeked_indent > expected_indent => {
                                    Some(peek_info)
                                }
                                GreaterOrEqual(expected_indent)
                                    if peeked_indent >= expected_indent =>
                                {
                                    Some(peek_info)
                                }
                                Equal(expected_indent) if peeked_indent == expected_indent => {
                                    Some(peek_info)
                                }
                                Greater if peeked_indent > start_indent => Some(peek_info),
                                Flexible => Some(peek_info),
                                _ => None,
                            }
                        }
                        _ => None,
                    }
                }
            }

            peek_count += 1;
        }

        None
    }

    // Consumes the next token depending on the rules of the current expression context
    //
    // It's expected that a peek has been performed (see peek_token_with_context) to check that the
    // current expression context allows for the token to be consumed.
    //
    // If the expression context allows linebreaks and its expected indentation is set to Greater,
    // and indentation is found, then the context will be updated to a) expect the new indentation,
    // and b) allow the start of map blocks.
    //
    // See also: `consume_until_token_with_context()`.
    fn consume_token_with_context(
        &mut self,
        context: &ExpressionContext,
    ) -> Option<(Token, ExpressionContext)> {
        let start_line = self.current_line_number();

        for token in &mut self.lexer {
            if !(token.is_whitespace() || token.is_newline()) {
                let is_indented_block = self.current_line_number() > start_line
                    && context.allow_linebreaks
                    && matches!(context.expected_indentation, Indentation::Greater);

                let new_context = if is_indented_block {
                    ExpressionContext {
                        expected_indentation: Indentation::Equal(self.current_indent()),
                        allow_map_block: true,
                        ..*context
                    }
                } else {
                    *context
                };

                return Some((token, new_context));
            }
        }

        None
    }

    // Consumes whitespace, comments, and newlines up until the next token
    //
    // See the description of `consume_token_with_context()` for more information.
    fn consume_until_token_with_context(
        &mut self,
        context: &ExpressionContext,
    ) -> Option<ExpressionContext> {
        let start_line = self.current_line_number();

        while let Some(peeked) = self.peek_token_n(0) {
            if peeked.is_whitespace() || peeked.is_newline() {
                self.lexer.next();
            } else {
                let is_indented_block = self.lexer.peek_line_number(0) > start_line
                    && context.allow_linebreaks
                    && matches!(context.expected_indentation, Indentation::Greater);

                let new_context = if is_indented_block {
                    ExpressionContext {
                        expected_indentation: Indentation::Equal(self.lexer.peek_indent(0)),
                        allow_map_block: true,
                        ..*context
                    }
                } else {
                    *context
                };

                return Some(new_context);
            }
        }

        None
    }

    // Peeks past whitespace on the same line until the next token is found
    fn peek_next_token_on_same_line(&mut self) -> Option<Token> {
        let mut peek_count = 0;

        while let Some(peeked) = self.peek_token_n(peek_count) {
            match peeked {
                token if token.is_whitespace() => {}
                token => return Some(token),
            }

            peek_count += 1;
        }

        None
    }

    // Consumes whitespace on the same line up until the next token
    fn consume_until_next_token_on_same_line(&mut self) {
        while let Some(peeked) = self.peek_token() {
            match peeked {
                token if token.is_whitespace() => {}
                _ => return,
            }

            self.lexer.next();
        }
    }

    // Consumes whitespace on the same line and returns the next token
    fn consume_next_token_on_same_line(&mut self) -> Option<Token> {
        while let Some(peeked) = self.peek_token() {
            match peeked {
                token if token.is_whitespace() => {}
                _ => return self.lexer.next(),
            }

            self.lexer.next();
        }

        None
    }

    fn frame(&self) -> Result<&Frame, ParserError> {
        match self.frame_stack.last() {
            Some(frame) => Ok(frame),
            None => Err(ParserError::new(
                InternalError::MissingFrame.into(),
                Span::default(),
            )),
        }
    }

    fn frame_mut(&mut self) -> Result<&mut Frame, ParserError> {
        match self.frame_stack.last_mut() {
            Some(frame) => Ok(frame),
            None => Err(ParserError::new(
                InternalError::MissingFrame.into(),
                Span::default(),
            )),
        }
    }
}

// Used by Parser::parse_expressions() to determine if comma-separated values should be stored in a
// Tuple or a TempTuple.
enum TempResult {
    No,
    Yes,
}

// The first operator that's above the pipe operator >> in precedence.
// Q: Why is this needed?
// A: Function calls without parentheses aren't currently treated as operators (a Call operator
//    with higher precedence than Pipe would allow this to go away, but would likely take quite a
//    bit of reworking. All calls to parse_call_args will need to reworked).
//    parse_call_args needs to parse arguments as expressions with a minimum precedence that
//    excludes piping, otherwise `f g >> x` would be parsed as `f (g >> x)` instead of `(f g) >> x`.
const MIN_PRECEDENCE_AFTER_PIPE: u8 = 3;

fn operator_precedence(op: Token) -> Option<(u8, u8)> {
    use Token::*;
    let priority = match op {
        Pipe => (1, 2),
        AddAssign | SubtractAssign => (4, MIN_PRECEDENCE_AFTER_PIPE),
        MultiplyAssign | DivideAssign | RemainderAssign => (6, 5),
        Or => (7, 8),
        And => (9, 10),
        // Chained comparisons require right-associativity
        Equal | NotEqual => (12, 11),
        Greater | GreaterOrEqual | Less | LessOrEqual => (14, 13),
        Add | Subtract => (15, 16),
        Multiply | Divide | Remainder => (17, 18),
        _ => return None,
    };
    Some(priority)
}

// Returned by Parser::peek_token_with_context()
#[derive(Debug)]
struct PeekInfo {
    token: Token,
    line: u32,
    indent: usize,
    peek_count: usize,
}

// Returned by Parser::parse_id_or_wildcard()
enum IdOrWildcard {
    Id(u32),
    Wildcard(Option<u32>),
}
