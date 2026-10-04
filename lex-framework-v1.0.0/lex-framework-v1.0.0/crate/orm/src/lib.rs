use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::collections::HashSet;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, Fields, GenericArgument, Lit, PathArguments, Type,
    parse_macro_input,
};

#[derive(Default)]
struct FieldOptions {
    primary_key: bool,
    unique: bool,
    index: bool,
    nullable: bool,
    soft_delete: bool,
    encrypted: bool,
    not_empty: bool,
    bool_true: bool,
    min: Option<f64>,
    max: Option<f64>,
    min_length: Option<usize>,
    max_length: Option<usize>,
    default: Option<String>,
    regex: Option<String>,
    depends_on: Option<syn::Ident>,
}

#[derive(Default)]
struct ModelOptions {
    ordering: Vec<String>,
    tenant_column: Option<String>,
    global: bool,
    soft_delete_column: Option<String>,
    version_column: Option<String>,
    unique: Vec<(String, Vec<String>)>,
}

#[derive(Default)]
struct RelationOptions {
    model: Option<syn::Path>,
    local: Option<String>,
    remote: Option<String>,
    many: bool,
    related_name: Option<String>,
    on_delete: Option<String>,
}

fn lit_from_expr(expr: Expr) -> syn::Result<Lit> {
    match expr {
        Expr::Lit(ExprLit { lit, .. }) => Ok(lit),
        other => Err(syn::Error::new_spanned(other, "expected a literal value")),
    }
}

fn safe_default_expression(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && !trimmed.contains(';')
        && !trimmed.contains("--")
        && !trimmed.contains("/*")
        && !trimmed.contains("*/")
        && !trimmed.contains('\0')
        && !trimmed.contains('\n')
        && !trimmed.contains('\r')
}

fn parse_field_options(attrs: &[Attribute]) -> syn::Result<FieldOptions> {
    let mut out = FieldOptions::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("field")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("primary_key") {
                out.primary_key = true;
                return Ok(());
            }
            if meta.path.is_ident("unique") {
                out.unique = true;
                return Ok(());
            }
            if meta.path.is_ident("index") {
                out.index = true;
                return Ok(());
            }
            if meta.path.is_ident("nullable") {
                out.nullable = true;
                return Ok(());
            }
            if meta.path.is_ident("soft_delete") {
                out.soft_delete = true;
                out.nullable = true;
                return Ok(());
            }
            if meta.path.is_ident("encrypted") {
                out.encrypted = true;
                return Ok(());
            }
            if meta.path.is_ident("not_empty") {
                out.not_empty = true;
                return Ok(());
            }
            if meta.path.is_ident("bool_true") {
                out.bool_true = true;
                return Ok(());
            }

            if ![
                "min",
                "max",
                "min_length",
                "max_length",
                "default",
                "regex",
                "depends_on",
            ]
            .iter()
            .any(|name| meta.path.is_ident(name))
            {
                return Err(meta.error("unsupported field option"));
            }
            let value = meta.value()?.parse::<Expr>()?;
            let lit = lit_from_expr(value)?;
            if meta.path.is_ident("min") {
                return match &lit {
                    Lit::Int(value) => {
                        out.min = Some(value.base10_parse::<f64>()?);
                        Ok(())
                    }
                    Lit::Float(value) => {
                        out.min = Some(value.base10_parse::<f64>()?);
                        Ok(())
                    }
                    _ => Err(syn::Error::new_spanned(&lit, "min must be numeric")),
                };
            }
            if meta.path.is_ident("max") {
                return match &lit {
                    Lit::Int(value) => {
                        out.max = Some(value.base10_parse::<f64>()?);
                        Ok(())
                    }
                    Lit::Float(value) => {
                        out.max = Some(value.base10_parse::<f64>()?);
                        Ok(())
                    }
                    _ => Err(syn::Error::new_spanned(&lit, "max must be numeric")),
                };
            }
            if meta.path.is_ident("min_length") {
                return match &lit {
                    Lit::Int(value) => {
                        out.min_length = Some(value.base10_parse()?);
                        Ok(())
                    }
                    _ => Err(syn::Error::new_spanned(
                        &lit,
                        "min_length must be an integer",
                    )),
                };
            }
            if meta.path.is_ident("max_length") {
                return match &lit {
                    Lit::Int(value) => {
                        out.max_length = Some(value.base10_parse()?);
                        Ok(())
                    }
                    _ => Err(syn::Error::new_spanned(
                        &lit,
                        "max_length must be an integer",
                    )),
                };
            }
            if meta.path.is_ident("default") {
                let default = match &lit {
                    Lit::Str(value) => value.value(),
                    Lit::Int(value) => value.base10_digits().to_owned(),
                    Lit::Float(value) => value.base10_digits().to_owned(),
                    Lit::Bool(value) => value.value.to_string(),
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "unsupported SQL default literal",
                        ));
                    }
                };
                if !safe_default_expression(&default) {
                    return Err(syn::Error::new_spanned(
                        &lit,
                        "SQL default contains a forbidden statement separator or comment",
                    ));
                }
                out.default = Some(default);
                return Ok(());
            }
            if meta.path.is_ident("regex") {
                return match &lit {
                    Lit::Str(value) => {
                        let pattern = value.value();
                        regex::Regex::new(&pattern).map_err(|error| {
                            syn::Error::new(value.span(), format!("invalid regex: {error}"))
                        })?;
                        out.regex = Some(pattern);
                        Ok(())
                    }
                    _ => Err(syn::Error::new_spanned(
                        &lit,
                        "regex must be a string literal",
                    )),
                };
            }
            if meta.path.is_ident("depends_on") {
                return match &lit {
                    Lit::Str(value) => {
                        out.depends_on =
                            Some(syn::parse_str::<syn::Ident>(&value.value()).map_err(|_| {
                                syn::Error::new(value.span(), "depends_on must name a Rust field")
                            })?);
                        Ok(())
                    }
                    _ => Err(syn::Error::new_spanned(
                        &lit,
                        "depends_on must be a string literal",
                    )),
                };
            }
            Err(meta.error("unsupported field attribute"))
        })?;
    }
    Ok(out)
}

fn parse_relation_options(attrs: &[Attribute]) -> syn::Result<Option<RelationOptions>> {
    let relation_attrs = attrs
        .iter()
        .filter(|attr| attr.path().is_ident("relation"))
        .collect::<Vec<_>>();
    if relation_attrs.is_empty() {
        return Ok(None);
    }
    if relation_attrs.len() != 1 {
        return Err(syn::Error::new_spanned(
            relation_attrs[1],
            "only one relation attribute is supported per field",
        ));
    }

    let mut out = RelationOptions::default();
    relation_attrs[0].parse_nested_meta(|meta| {
        if meta.path.is_ident("many") {
            out.many = true;
            return Ok(());
        }
        if meta.path.is_ident("model") {
            let value = meta.value()?.parse::<syn::LitStr>()?;
            out.model = Some(syn::parse_str::<syn::Path>(&value.value()).map_err(|_| {
                syn::Error::new(value.span(), "relation model must be a Rust type path")
            })?);
            return Ok(());
        }
        if meta.path.is_ident("local") {
            let value = meta.value()?.parse::<syn::LitStr>()?;
            validate_identifier(&value.value(), value.span())?;
            out.local = Some(value.value());
            return Ok(());
        }
        if meta.path.is_ident("remote") {
            let value = meta.value()?.parse::<syn::LitStr>()?;
            validate_identifier(&value.value(), value.span())?;
            out.remote = Some(value.value());
            return Ok(());
        }
        if meta.path.is_ident("related_name") {
            let value = meta.value()?.parse::<syn::LitStr>()?;
            validate_identifier(&value.value(), value.span())?;
            out.related_name = Some(value.value());
            return Ok(());
        }
        if meta.path.is_ident("on_delete") {
            let value = meta.value()?.parse::<syn::LitStr>()?;
            let normalized = value.value().to_ascii_lowercase();
            if !matches!(
                normalized.as_str(),
                "cascade" | "restrict" | "set_null" | "set_default" | "no_action"
            ) {
                return Err(syn::Error::new(
                    value.span(),
                    "on_delete must be cascade, restrict, set_null, set_default, or no_action",
                ));
            }
            out.on_delete = Some(normalized);
            return Ok(());
        }
        Err(meta.error("unsupported relation attribute"))
    })?;

    if out.model.is_none() || out.local.is_none() || out.remote.is_none() {
        return Err(syn::Error::new_spanned(
            relation_attrs[0],
            "relation requires model, local, and remote",
        ));
    }
    Ok(Some(out))
}

fn parse_model_options(attrs: &[Attribute]) -> syn::Result<ModelOptions> {
    let mut out = ModelOptions::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("model")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("unique") {
                let mut name = None;
                let mut fields = None;
                meta.parse_nested_meta(|option| {
                    let value = option.value()?.parse::<syn::LitStr>()?;
                    if option.path.is_ident("name") {
                        validate_identifier(&value.value(), value.span())?;
                        name = Some(value.value());
                    } else if option.path.is_ident("fields") {
                        let columns = value
                            .value()
                            .split(',')
                            .map(str::trim)
                            .map(str::to_owned)
                            .collect::<Vec<_>>();
                        for column in &columns {
                            validate_identifier(column, value.span())?;
                        }
                        fields = Some(columns);
                    } else {
                        return Err(option.error("unique supports name and fields"));
                    }
                    Ok(())
                })?;
                out.unique.push((
                    name.ok_or_else(|| meta.error("unique requires name"))?,
                    fields.ok_or_else(|| meta.error("unique requires fields"))?,
                ));
                return Ok(());
            }
            if meta.path.is_ident("global") {
                out.global = true;
                return Ok(());
            }
            if meta.path.is_ident("ordering") {
                let value = meta.value()?.parse::<syn::LitStr>()?;
                let mut ordering = Vec::new();
                for item in value.value().split(',') {
                    let item = item.trim();
                    if item.is_empty() {
                        continue;
                    }
                    let field = item.strip_prefix('-').unwrap_or(item);
                    validate_identifier(field, value.span())?;
                    ordering.push(item.to_owned());
                }
                if ordering.is_empty() {
                    return Err(syn::Error::new(
                        value.span(),
                        "ordering must contain at least one field",
                    ));
                }
                out.ordering = ordering;
                return Ok(());
            }
            if meta.path.is_ident("tenant") {
                let value = meta.value()?.parse::<syn::LitStr>()?;
                validate_identifier(&value.value(), value.span())?;
                out.tenant_column = Some(value.value());
                return Ok(());
            }
            if meta.path.is_ident("soft_delete") {
                let value = meta.value()?.parse::<syn::LitStr>()?;
                validate_identifier(&value.value(), value.span())?;
                out.soft_delete_column = Some(value.value());
                return Ok(());
            }
            if meta.path.is_ident("version") {
                let value = meta.value()?.parse::<syn::LitStr>()?;
                validate_identifier(&value.value(), value.span())?;
                out.version_column = Some(value.value());
                return Ok(());
            }
            Err(meta.error("unsupported model attribute"))
        })?;
    }
    if out.global && out.tenant_column.is_some() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "model cannot be both global and explicitly tenant-scoped",
        ));
    }
    Ok(out)
}

fn table_name(attrs: &[Attribute], struct_name: &syn::Ident) -> syn::Result<String> {
    let mut name = None;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("table")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let value = meta.value()?.parse::<syn::LitStr>()?;
                name = Some(value.value());
                Ok(())
            } else {
                Err(meta.error("table supports only name = \"...\""))
            }
        })?;
    }
    let name = name.unwrap_or_else(|| {
        let input = struct_name.to_string();
        let mut out = String::new();
        for (index, ch) in input.chars().enumerate() {
            if ch.is_ascii_uppercase() && index != 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        }
        out
    });
    validate_identifier(&name, struct_name.span())?;
    Ok(name)
}

fn validate_identifier(value: &str, span: proc_macro2::Span) -> syn::Result<()> {
    let mut chars = value.chars();
    if value.is_empty()
        || value.len() > 63
        || !matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        || !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        return Err(syn::Error::new(
            span,
            format!("unsafe or overlong PostgreSQL identifier: {value}"),
        ));
    }
    Ok(())
}

fn is_option(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Option"))
}

fn normalized_type(ty: &Type) -> String {
    quote!(#ty).to_string().replace(' ', "")
}

fn is_choice_field(ty: &Type) -> bool {
    named_generic_inner(ty, "ChoiceField").is_some()
}

fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    })
}

fn named_generic_inner<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let ty = option_inner_type(ty).unwrap_or(ty);
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    })
}

fn sql_type(ty: &Type) -> &'static str {
    if let Some(inner) = option_inner_type(ty) {
        return sql_type(inner);
    }
    let value = normalized_type(ty);
    if value == "Vec<u8>" || value.ends_with("::Vec<u8>") {
        return "BYTEA";
    }
    let name = match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    };
    match name.as_deref() {
        Some("Uuid" | "UUIDField") => "UUID",
        Some("DateTime" | "DateTimeField") => "TIMESTAMPTZ",
        Some("NaiveDate") => "DATE",
        Some("NaiveTime") => "TIME",
        Some("Decimal" | "DecimalField") => "NUMERIC",
        Some("bool" | "BooleanField") => "BOOLEAN",
        Some("i16") => "SMALLINT",
        Some("i32" | "IntegerField") => "INTEGER",
        Some("i64") => "BIGINT",
        Some("f32") => "REAL",
        Some("f64" | "FloatField") => "DOUBLE PRECISION",
        Some(
            "BinaryField" | "ImageField" | "AudioField" | "VideoField" | "PdfField"
            | "DocumentField",
        ) => "BYTEA",
        Some("JsonField" | "ManyToMany") => "JSONB",
        Some("ForeignKey") => "INTEGER",
        _ => "TEXT",
    }
}

#[proc_macro_derive(Model, attributes(table, model, field, relation, timestamps))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_model(input) {
        Ok(tokens) => match resolve_runtime_paths(tokens) {
            Ok(tokens) => tokens.into(),
            Err(error) => error.into_compile_error().into(),
        },
        Err(error) => error.into_compile_error().into(),
    }
}

// Resolve the consumer's dependency name, including aliases and facade-only applications.
// Rewriting parsed paths leaves model fields and string literals untouched.
fn resolve_runtime_paths(
    tokens: proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    use proc_macro_crate::{FoundCrate, crate_name};
    use syn::visit_mut::{VisitMut, visit_path_mut};
    fn root(found: FoundCrate) -> proc_macro2::TokenStream {
        match found {
            FoundCrate::Itself => quote!(crate),
            FoundCrate::Name(name) => {
                let ident = format_ident!("{}", name);
                quote!(::#ident)
            }
        }
    }
    let runtime = if let Ok(found) =
        crate_name("lex-db-controller").or_else(|_| crate_name("db_controller"))
    {
        // Cargo chooses the consumer's dependency key, including its normalized default
        // package name. Never replace it with the library target's name.
        root(found)
    } else if let Ok(found) = crate_name("lex-framework").or_else(|_| crate_name("lex_framework")) {
        let facade = root(found);
        quote!(#facade::db_controller)
    } else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "Model requires a dependency on lex_framework or db_controller (renamed dependencies are supported)",
        ));
    };
    struct Paths {
        runtime: syn::Path,
    }
    impl VisitMut for Paths {
        fn visit_type_path_mut(&mut self, value: &mut syn::TypePath) {
            // Qualified paths retain a separate trait boundary; extending the crate prefix
            // must move that boundary as well as the path's segments.
            let before = value.path.segments.len();
            syn::visit_mut::visit_type_path_mut(self, value);
            if let Some(qualifier) = &mut value.qself
                && qualifier.position > 0
            {
                qualifier.position += value.path.segments.len().saturating_sub(before);
            }
        }
        fn visit_expr_path_mut(&mut self, value: &mut syn::ExprPath) {
            let before = value.path.segments.len();
            syn::visit_mut::visit_expr_path_mut(self, value);
            if let Some(qualifier) = &mut value.qself
                && qualifier.position > 0
            {
                qualifier.position += value.path.segments.len().saturating_sub(before);
            }
        }
        fn visit_item_use_mut(&mut self, item: &mut syn::ItemUse) {
            if matches!(&item.tree, syn::UseTree::Path(path) if path.ident == "db_controller") {
                let runtime = &self.runtime;
                *item = syn::parse_quote!(use #runtime::sqlx::Row as _;);
            }
            syn::visit_mut::visit_item_use_mut(self, item);
        }
        fn visit_path_mut(&mut self, path: &mut syn::Path) {
            visit_path_mut(self, path);
            let Some(first) = path.segments.first() else {
                return;
            };
            let replacement = if first.ident == "db_controller" {
                Some(self.runtime.clone())
            } else if first.ident == "serde_json" {
                let runtime = &self.runtime;
                Some(syn::parse_quote!(#runtime::serde_json))
            } else {
                None
            };
            if let Some(mut replacement) = replacement {
                replacement
                    .segments
                    .extend(path.segments.iter().skip(1).cloned());
                *path = replacement;
            }
        }
    }
    let mut file: syn::File = syn::parse2(tokens)?;
    Paths {
        runtime: syn::parse2(runtime)?,
    }
    .visit_file_mut(&mut file);
    Ok(quote!(#file))
}

fn expand_model(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let model_options = parse_model_options(&input.attrs)?;
    let struct_ident = input.ident;
    let table = table_name(&input.attrs, &struct_ident)?;
    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields.named.into_iter().collect::<Vec<_>>(),
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "Model supports only structs with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new(
                struct_ident.span(),
                "Model supports only structs",
            ));
        }
    };

    let field_names = fields
        .iter()
        .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
        .collect::<HashSet<_>>();

    for ordering in &model_options.ordering {
        let field = ordering.strip_prefix('-').unwrap_or(ordering);
        if !field_names.contains(field) && !matches!(field, "id" | "created_at" | "updated_at") {
            return Err(syn::Error::new(
                struct_ident.span(),
                format!("model ordering field does not exist: {field}"),
            ));
        }
    }
    for (label, field) in [
        ("tenant", model_options.tenant_column.as_deref()),
        ("soft_delete", model_options.soft_delete_column.as_deref()),
        ("version", model_options.version_column.as_deref()),
    ] {
        if let Some(field) = field
            && !field_names.contains(field)
        {
            return Err(syn::Error::new(
                struct_ident.span(),
                format!("model {label} field does not exist: {field}"),
            ));
        }
    }

    let ordering_values = model_options
        .ordering
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let tenant_column = if model_options.global {
        None
    } else {
        model_options
            .tenant_column
            .as_deref()
            .or_else(|| field_names.contains("tenant_id").then_some("tenant_id"))
    };
    let soft_delete_column = model_options
        .soft_delete_column
        .as_deref()
        .or_else(|| field_names.contains("deleted_at").then_some("deleted_at"));
    let version_column = model_options
        .version_column
        .as_deref()
        .or_else(|| field_names.contains("version").then_some("version"));
    let tenant_column_tokens = match tenant_column {
        Some(value) => quote!(Some(#value)),
        None => quote!(None),
    };
    let soft_delete_column_tokens = match soft_delete_column {
        Some(value) => quote!(Some(#value)),
        None => quote!(None),
    };
    let version_column_tokens = match version_column {
        Some(value) => quote!(Some(#value)),
        None => quote!(None),
    };

    let mut column_nullable = std::collections::HashMap::new();
    for field in &fields {
        if parse_relation_options(&field.attrs)?.is_none()
            && let Some(ident) = &field.ident
        {
            let options = parse_field_options(&field.attrs)?;
            column_nullable.insert(ident.to_string(), options.nullable || is_option(&field.ty));
        }
    }

    let mut schema_entries = Vec::new();
    let mut map_entries = Vec::new();
    let mut row_entries = Vec::new();
    let mut sqlite_row_entries = Vec::new();
    let mut index_entries = Vec::new();
    let mut validations = Vec::new();
    let mut relation_entries = Vec::new();
    let mut relation_accessors = Vec::new();
    let mut field_descriptors = Vec::new();
    let mut unique_entries = Vec::new();
    let mut constraint_names = HashSet::new();
    for (name, columns) in &model_options.unique {
        if !constraint_names.insert(name) {
            return Err(syn::Error::new(
                struct_ident.span(),
                "duplicate unique constraint name",
            ));
        }
        let mut seen = HashSet::new();
        for column in columns {
            if !field_names.contains(column.as_str()) || !seen.insert(column) {
                return Err(syn::Error::new(
                    struct_ident.span(),
                    format!("unique constraint references an unknown or repeated field: {column}"),
                ));
            }
        }
        unique_entries.push(quote! { db_controller::UniqueConstraintSpec { name: #name, columns: &[#(#columns),*] } });
    }
    let mut primary_key = None::<String>;
    let mut has_soft_delete = false;

    for field in fields {
        let ident = field
            .ident
            .ok_or_else(|| syn::Error::new_spanned(&field.ty, "field name required"))?;
        let name = ident.to_string();
        validate_identifier(&name, ident.span())?;

        if let Some(relation) = parse_relation_options(&field.attrs)? {
            let local = relation.local.as_deref().unwrap_or_default();
            if local != "id" && !field_names.contains(local) {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("relation local field does not exist: {local}"),
                ));
            }
            let model = relation.model.as_ref().expect("validated relation model");
            let remote = relation
                .remote
                .as_deref()
                .expect("validated relation remote");
            let many = relation.many;
            let cardinality = if many {
                quote!(db_controller::RelationCardinality::Many)
            } else {
                quote!(db_controller::RelationCardinality::One)
            };
            let on_delete = match relation.on_delete.as_deref().unwrap_or("restrict") {
                "cascade" => quote!(db_controller::OnDelete::Cascade),
                "set_null" => quote!(db_controller::OnDelete::SetNull),
                "set_default" => quote!(db_controller::OnDelete::SetDefault),
                "no_action" => quote!(db_controller::OnDelete::NoAction),
                _ => quote!(db_controller::OnDelete::Restrict),
            };
            let related_name = match relation.related_name.as_deref() {
                Some(value) => quote!(Some(#value)),
                None => quote!(None),
            };
            let nullable = column_nullable.get(local).copied().unwrap_or(false);
            if relation.on_delete.as_deref() == Some("set_null") && !nullable {
                return Err(syn::Error::new(
                    ident.span(),
                    format!(
                        "relation {name} uses on_delete = set_null but local field {local} is not nullable"
                    ),
                ));
            }
            relation_entries.push(quote! {
                db_controller::RelationSpec {
                    field_name: #name,
                    target_table: <#model as db_controller::Entity>::table_name(),
                    local_column: #local,
                    remote_column: #remote,
                    cardinality: #cardinality,
                    nullable: #nullable,
                    related_name: #related_name,
                    on_delete: #on_delete,
                }
            });
            let accessor = format_ident!("{}_relation", name);
            relation_accessors.push(quote! {
                pub fn #accessor() -> db_controller::RelationSpec {
                    db_controller::RelationSpec {
                        field_name: #name,
                        target_table: <#model as db_controller::Entity>::table_name(),
                        local_column: #local,
                        remote_column: #remote,
                        cardinality: #cardinality,
                        nullable: #nullable,
                        related_name: #related_name,
                        on_delete: #on_delete,
                    }
                }
            });
            row_entries.push(quote! { #ident: std::default::Default::default() });
            sqlite_row_entries.push(quote! { #ident: std::default::Default::default() });
            continue;
        }

        let options = parse_field_options(&field.attrs)?;
        let field_type = &field.ty;
        let field_const = format_ident!("{}", name.to_ascii_uppercase());
        field_descriptors.push(quote! {
            pub const #field_const: db_controller::Field<#field_type> = db_controller::Field::new(#name);
        });
        if let Some(dependency) = &options.depends_on
            && !field_names.contains(&dependency.to_string())
        {
            return Err(syn::Error::new(
                dependency.span(),
                format!("depends_on field does not exist: {dependency}"),
            ));
        }
        if options.primary_key && (options.nullable || is_option(&field.ty)) {
            return Err(syn::Error::new(
                ident.span(),
                "primary key fields cannot be nullable",
            ));
        }
        if options.primary_key && primary_key.replace(name.clone()).is_some() {
            return Err(syn::Error::new(
                ident.span(),
                "only one primary key is supported",
            ));
        }
        if options.soft_delete {
            has_soft_delete = true;
        }

        let sql = sql_type(&field.ty);
        let nullable = options.nullable || is_option(&field.ty);
        let unique = options.unique;
        let primary = options.primary_key;
        let encrypted = options.encrypted;
        let default_expr = match options.default.clone() {
            Some(value) => quote!(Some(#value.to_owned())),
            None => quote!(None),
        };
        schema_entries.push(quote! {
            schema.insert(
                #name.to_owned(),
                db_controller::ColumnSpec {
                    sql_type: #sql.to_owned(),
                    nullable: #nullable,
                    unique: #unique,
                    primary_key: #primary,
                    default: #default_expr,
                    encrypted: #encrypted,
                },
            );
        });

        let optional = is_option(&field.ty);
        if is_choice_field(&field.ty) {
            return Err(syn::Error::new_spanned(
                &field.ty,
                "ChoiceField cannot be persisted because its choices are runtime state; use a typed enum for database-backed model fields",
            ));
        }

        if let Some(json_inner) = named_generic_inner(&field.ty, "JsonField") {
            if optional {
                row_entries.push(quote! {
                    #ident: row
                        .try_get::<Option<db_controller::sqlx::types::Json<#json_inner>>, _>(#name)?
                        .map(|value| db_controller::JsonField(value.0))
                });
            } else {
                row_entries.push(quote! {
                    #ident: db_controller::JsonField(
                        row.try_get::<db_controller::sqlx::types::Json<#json_inner>, _>(#name)?.0
                    )
                });
            }
        } else {
            row_entries.push(quote! { #ident: row.try_get(#name)? });
        }

        if matches!(sql, "UUID" | "NUMERIC") {
            let decoding = if optional {
                quote! { row.try_get::<Option<String>, _>(#name)?.map(|value| value.parse()).transpose() }
            } else {
                quote! { row.try_get::<String, _>(#name)?.parse() }
            };
            sqlite_row_entries.push(quote! {
                #ident: #decoding.map_err(|error| db_controller::sqlx::Error::ColumnDecode {
                    index: #name.to_owned(), source: Box::new(error),
                })?
            });
        } else {
            sqlite_row_entries.push(row_entries.last().expect("generated row entry").clone());
        }

        map_entries.push(quote! {
            if let Ok(value) = serde_json::to_value(&self.#ident) {
                map.insert(#name.to_owned(), value);
            }
        });

        if options.unique {
            let constraint_name = format!("uq_{}_{}", table, name);
            validate_identifier(&constraint_name, ident.span())?;
            unique_entries.push(quote! {
                db_controller::UniqueConstraintSpec {
                    name: #constraint_name,
                    columns: &[#name],
                }
            });
        }

        if options.index {
            let index_name = format!("idx_{}_{}", table, name);
            validate_identifier(&index_name, ident.span())?;
            index_entries.push(quote! {
                db_controller::IndexSpec {
                    name: #index_name,
                    columns: &[#name],
                    unique: false,
                    method: db_controller::IndexMethod::BTree,
                }
            });
        }

        let mut checks = Vec::new();
        if options.not_empty {
            checks.push(quote! {
                if let Some(value) = field_value.as_str() {
                    if value.trim().is_empty() {
                        return Err(format!("{} must not be empty", #name));
                    }
                }
            });
        }
        if options.bool_true {
            checks.push(quote! {
                if field_value.as_bool() == Some(false) {
                    return Err(format!("{} must be true", #name));
                }
            });
        }
        if let Some(min) = options.min {
            checks.push(quote! {
                if let Some(value) = field_value.as_f64() {
                    if value < #min {
                        return Err(format!("{} must be at least {}", #name, #min));
                    }
                }
            });
        }
        if let Some(max) = options.max {
            checks.push(quote! {
                if let Some(value) = field_value.as_f64() {
                    if value > #max {
                        return Err(format!("{} must be at most {}", #name, #max));
                    }
                }
            });
        }
        if let Some(min) = options.min_length {
            checks.push(quote! {
                if let Some(value) = field_value.as_str() {
                    if value.chars().count() < #min {
                        return Err(format!("{} is shorter than {} characters", #name, #min));
                    }
                }
            });
        }
        if let Some(max) = options.max_length {
            checks.push(quote! {
                if let Some(value) = field_value.as_str() {
                    if value.chars().count() > #max {
                        return Err(format!("{} is longer than {} characters", #name, #max));
                    }
                }
            });
        }
        if let Some(pattern) = options.regex {
            checks.push(quote! {
                if let Some(value) = field_value.as_str() {
                    if !db_controller::regex_is_match(#pattern, value)
                        .map_err(|error| format!("invalid validator regex for {}: {}", #name, error))?
                    {
                        return Err(format!("{} has invalid format", #name));
                    }
                }
            });
        }
        if let Some(dependency) = options.depends_on {
            checks.push(quote! {
                if !field_value.is_null() {
                    let dependency = serde_json::to_value(&self.#dependency)
                        .map_err(|error| format!("failed to validate {} dependency: {}", #name, error))?;
                    if dependency.is_null() {
                        return Err(format!("{} requires {}", #name, stringify!(#dependency)));
                    }
                }
            });
        }

        if !checks.is_empty() {
            let field_value = quote! {
                serde_json::to_value(&self.#ident)
                    .map_err(|error| format!("failed to validate {}: {}", #name, error))?
            };
            validations.push(quote! {
                let field_value = #field_value;
                if !field_value.is_null() {
                    #(#checks)*
                }
            });
        }
    }

    let primary_key = primary_key.unwrap_or_else(|| "id".to_owned());
    let soft_delete_impl = if has_soft_delete {
        quote! { impl db_controller::SoftDelete for #struct_ident {} }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl db_controller::Entity for #struct_ident {
            fn table_name() -> &'static str {
                #table
            }

            fn primary_key() -> &'static str {
                #primary_key
            }

            fn schema() -> std::collections::HashMap<String, db_controller::ColumnSpec> {
                let mut schema = std::collections::HashMap::new();
                #(#schema_entries)*
                schema
            }

            fn to_map(&self) -> std::collections::HashMap<String, serde_json::Value> {
                let mut map = std::collections::HashMap::new();
                #(#map_entries)*
                map
            }

            fn validate_model(&self) -> Result<(), String> {
                self.validate()
            }

            fn typed_indexes() -> Vec<db_controller::IndexSpec> {
                Vec::from([#(#index_entries),*])
            }

            fn relations() -> Vec<db_controller::RelationSpec> {
                Vec::from([#(#relation_entries),*])
            }

            fn unique_constraints() -> Vec<db_controller::UniqueConstraintSpec> {
                Vec::from([#(#unique_entries),*])
            }

            fn default_ordering() -> Vec<&'static str> {
                vec![#(#ordering_values),*]
            }

            fn tenant_column() -> Option<&'static str> {
                #tenant_column_tokens
            }

            fn soft_delete_column() -> Option<&'static str> {
                #soft_delete_column_tokens
            }

            fn version_column() -> Option<&'static str> {
                #version_column_tokens
            }
        }

        impl<'r> db_controller::sqlx::FromRow<'r, db_controller::sqlx::postgres::PgRow> for #struct_ident {
            fn from_row(
                row: &'r db_controller::sqlx::postgres::PgRow,
            ) -> Result<Self, db_controller::sqlx::Error> {
                use db_controller::sqlx::Row as _;
                Ok(Self {
                    #(#row_entries,)*
                })
            }
        }

        impl<'r> db_controller::sqlx::FromRow<'r, db_controller::sqlx::sqlite::SqliteRow> for #struct_ident {
            fn from_row(
                row: &'r db_controller::sqlx::sqlite::SqliteRow,
            ) -> Result<Self, db_controller::sqlx::Error> {
                use db_controller::sqlx::Row as _;
                Ok(Self {
                    #(#sqlite_row_entries,)*
                })
            }
        }

        #soft_delete_impl

        impl #struct_ident {
            #(#field_descriptors)*
            #(#relation_accessors)*

            /// Django-style manager entry point backed by the canonical ModelManager/QuerySet engine.
            pub fn objects(pool: db_controller::sqlx::PgPool) -> db_controller::ModelManager<Self> {
                db_controller::ModelManager::new(pool)
            }

            /// SQLite manager entry point. This uses the same QuerySet surface as PostgreSQL
            /// while applying SQLite-compatible SQL at execution time.
            pub fn objects_sqlite(
                pool: db_controller::sqlx::SqlitePool,
            ) -> db_controller::SqliteModelManager<Self> {
                db_controller::SqliteModelManager::new(pool)
            }

            /// Django-like local-development bootstrap: create the SQLite table/indexes when
            /// missing and return the normal SQLite QuerySet manager.
            pub async fn objects_sqlite_auto(
                pool: db_controller::sqlx::SqlitePool,
            ) -> Result<db_controller::SqliteModelManager<Self>, db_controller::ModelError> {
                db_controller::SqliteModelManager::initialize(pool).await
            }

            /// Run generated model validation before persistence or explicit application checks.
            pub fn full_clean(&self) -> Result<(), String> {
                self.validate()
            }

            pub fn validate(&self) -> Result<(), String> {
                #(#validations)*
                Ok(())
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn valid_model_metadata_expands() {
        let input: DeriveInput = parse_quote! {
            #[table(name = "users")]
            #[model(ordering = "-created_at", tenant = "tenant_id", version = "version")]
            struct User {
                #[field(primary_key)]
                id: uuid::Uuid,
                tenant_id: uuid::Uuid,
                #[field(unique, not_empty, max_length = 320)]
                email: String,
                version: i64,
                created_at: chrono::DateTime<chrono::Utc>,
            }
        };
        assert!(expand_model(input).is_ok());
    }

    #[test]
    fn duplicate_primary_keys_are_rejected_at_macro_expansion() {
        let input: DeriveInput = parse_quote! {
            struct Broken {
                #[field(primary_key)]
                first: i64,
                #[field(primary_key)]
                second: i64,
            }
        };
        let error = expand_model(input).expect_err("duplicate primary keys must fail");
        assert!(error.to_string().contains("only one primary key"));
    }

    #[test]
    fn model_scope_metadata_must_reference_real_fields() {
        let input: DeriveInput = parse_quote! {
            #[model(tenant = "missing_tenant")]
            struct Broken {
                id: i64,
            }
        };
        let error = expand_model(input).expect_err("missing tenant metadata must fail");
        assert!(error.to_string().contains("tenant field does not exist"));
    }

    #[test]
    fn relation_local_columns_must_exist() {
        let input: DeriveInput = parse_quote! {
            struct BrokenRelation {
                id: i64,
                #[relation(model = "Other", local = "missing_id", remote = "id")]
                other: Option<Other>,
            }
        };
        let error = expand_model(input).expect_err("missing relation local column must fail");
        assert!(
            error
                .to_string()
                .contains("relation local field does not exist")
        );
    }

    #[test]
    fn unsafe_sql_defaults_are_rejected_during_macro_expansion() {
        let input: DeriveInput = parse_quote! {
            struct BrokenDefault {
                #[field(default = "now(); DROP TABLE users")]
                created_at: chrono::DateTime<chrono::Utc>,
            }
        };
        let error = expand_model(input).expect_err("unsafe SQL default must fail");
        assert!(
            error
                .to_string()
                .contains("SQL default contains a forbidden")
        );
    }
}
