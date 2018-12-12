use super::{
    graphql_scalar_type_to_rust_type, ident, quote_ident, type_name, AddToOutput, Output, TypeType,
};
use crate::nullable_type::NullableType;
use graphql_parser::{
    query::{Name, Type},
    schema::*,
};
use heck::{CamelCase, SnakeCase};
use lazy_static::lazy_static;
use proc_macro2::TokenStream;
use quote::quote;
use regex::Regex;
use syn::Ident;

pub fn gen_juniper_code(doc: Document, out: &mut Output) {
    gen_doc(doc, out);
}

fn gen_doc(doc: Document, out: &mut Output) {
    for def in doc.definitions {
        gen_def(def, out);
    }
}

fn gen_def(def: Definition, out: &mut Output) {
    use graphql_parser::schema::Definition::*;

    match def {
        DirectiveDefinition(_) => todo!("directive definition"),
        SchemaDefinition(schema_def) => gen_schema_def(schema_def, out),
        TypeDefinition(type_def) => gen_type_def(type_def, out),
        TypeExtension(_) => todo!("type extension"),
    }
}

fn gen_schema_def(schema_def: SchemaDefinition, out: &mut Output) {
    // TODO: use
    //   position
    //   directives
    //   subscription

    let query = match schema_def.query {
        Some(query) => ident(query),
        None => panic!("Juniper requires that the schema type has a query"),
    };

    let mutation = match schema_def.mutation {
        Some(mutation) => quote_ident(mutation),
        None => quote! { juniper::EmptyMutation<()> },
    };

    (quote! {
        /// The GraphQL schema type generated by `juniper-from-schema`.
        pub type Schema = juniper::RootNode<'static, #query, #mutation>;
    })
    .add_to(out)
}

fn gen_type_def(type_def: TypeDefinition, out: &mut Output) {
    use graphql_parser::schema::TypeDefinition::*;

    match type_def {
        Enum(enum_type) => gen_enum_type(enum_type, out),
        Object(obj_type) => gen_obj_type(obj_type, out),
        Scalar(scalar_type) => gen_scalar_type(scalar_type, out),
        InputObject(_) => todo!("input object"),
        Interface(_) => todo!("interface"),
        Union(_) => todo!("union"),
    }
}

fn gen_enum_type(enum_type: EnumType, out: &mut Output) {
    // TODO: use
    //   position
    //   description
    //   directives

    let name = ident(enum_type.name.to_camel_case());

    let values = gen_with(gen_enum_value, enum_type.values, &out);

    (quote! {
        /// GraphQL enum generated by `juniper-from-schema`.
        #[derive(juniper::GraphQLEnum, Debug, Eq, PartialEq, Copy, Clone, Hash)]
        pub enum #name {
            #values
        }
    })
    .add_to(out)
}

fn gen_enum_value(enum_type: EnumValue, out: &mut Output) {
    // TODO: use
    //   position
    //   description
    //   directives

    let graphql_name = enum_type.name;
    let name = ident(graphql_name.to_camel_case());
    (quote! {
        /// GraphQL enum variant generated by `juniper-from-schema`.
        #[graphql(name=#graphql_name)]
        #name,
    })
    .add_to(out)
}

fn gen_scalar_type(scalar_type: ScalarType, out: &mut Output) {
    // TODO: use
    //   position
    //   directives

    match &*scalar_type.name {
        "Date" => {}
        "DateTime" => {}
        name => {
            let name = ident(name);
            let description = scalar_type
                .description
                .map(|desc| quote! { description: #desc })
                .unwrap_or(quote! {});

            (quote! {
                /// Custom scalar type generated by `juniper-from-schema`.
                pub struct #name(pub String);

                juniper::graphql_scalar!(#name {
                    #description

                    resolve(&self) -> juniper::Value {
                        juniper::Value::string(&self.0)
                    }

                    from_input_value(v: &InputValue) -> Option<#name> {
                        v.as_string_value().map(|s| #name::new(s.to_owned()))
                    }
                });
            })
            .add_to(out);
        }
    };
}

fn gen_obj_type(obj_type: ObjectType, out: &mut Output) {
    // TODO: Use
    //   implements_interface
    //   directives

    let struct_name = ident(obj_type.name);

    let trait_name = ident(format!("{}Fields", struct_name));

    let field_tokens = obj_type
        .fields
        .into_iter()
        .map(|field| gen_field(field, &out))
        .collect::<Vec<_>>();

    let trait_methods = field_tokens
        .iter()
        .map(|field| {
            let field_name = field.field_method.clone();
            let field_type = field.field_type.clone();

            let args = field.args.clone();

            match field.type_type {
                TypeType::Scalar => {
                    quote! {
                        /// Field method generated by `juniper-from-schema`.
                        fn #field_name<'a>(&self, executor: &Executor<'a, Context>, #(#args),*) -> FieldResult<#field_type>;
                    }
                },
                TypeType::Type => {
                    let query_trail_type = ident(&field.inner_type);
                    let trail = quote! { &QueryTrail<'a, #query_trail_type, Walked> };
                    quote! {
                        /// Field method generated by `juniper-from-schema`.
                        fn #field_name<'a>(
                            &self,
                            executor: &Executor<'a, Context>,
                            trail: #trail, #(#args),*
                        ) -> FieldResult<#field_type>;
                    }
                }
            }
        });

    (quote! {
        /// Trait for GraphQL field methods generated by `juniper-from-schema`.
        pub trait #trait_name {
            #(#trait_methods)*
        }
    })
    .add_to(out);

    let fields = field_tokens
        .iter()
        .map(|field| {
            let field_name = field.name.clone();
            let field_type = field.field_type.clone();
            let args = field.args.clone();
            let field_method = field.field_method.clone();
            let params = field.params.clone();
            let description = field
                .description
                .clone()
                .map(|d| quote! { as #d })
                .unwrap_or(empty_token_stream());

            let body = match field.type_type {
                TypeType::Scalar => {
                    quote! {
                        <#struct_name as self::#trait_name>::#field_method(&self, &executor, #(#params),*)
                    }
                },
                TypeType::Type => {
                    let query_trail_type = ident(&field.inner_type);
                    quote! {
                        let look_ahead = executor.look_ahead();
                        let trail = look_ahead.make_query_trail::<#query_trail_type>();
                        <#struct_name as self::#trait_name>::#field_method(&self, &executor, &trail, #(#params),*)
                    }
                }
            };

            quote! {
                field #field_name(&executor, #(#args),*) -> juniper::FieldResult<#field_type> #description {
                    #body
                }
            }
        });

    let description = obj_type
        .description
        .map(|d| quote! { description: #d })
        .unwrap_or(empty_token_stream());

    (quote! {
        juniper::graphql_object!(#struct_name: Context |&self| {
            #description
            #(#fields)*
        });
    })
    .add_to(out);
}

fn empty_token_stream() -> TokenStream {
    quote! {}
}

#[derive(Debug)]
struct FieldTokens {
    name: Ident,
    args: Vec<TokenStream>,
    field_type: TokenStream,
    field_method: Ident,
    params: Vec<Ident>,
    description: Option<String>,
    type_type: TypeType,
    inner_type: Name,
}

fn gen_field(field: Field, out: &Output) -> FieldTokens {
    // TODO: Use
    //   directives

    let name = ident(field.name);

    let inner_type = type_name(&field.field_type).to_camel_case();

    let description = field.description.clone();

    let attributes = field
        .description
        .map(|d| parse_attributes(&d))
        .unwrap_or_else(|| Attributes::default());

    let (field_type, type_type) = gen_field_type(
        field.field_type,
        &FieldTypeDestination::Return(attributes),
        out,
    );

    let field_method = ident(format!("field_{}", name.to_string().to_snake_case()));

    let args_names_and_types = field
        .arguments
        .into_iter()
        .map(|x| argument_to_name_and_rust_type(x, out))
        .collect::<Vec<_>>();

    let args = args_names_and_types
        .iter()
        .map(|(arg, arg_type)| {
            let arg = ident(arg);
            quote! { #arg: #arg_type }
        })
        .collect::<Vec<_>>();

    let params = args_names_and_types
        .iter()
        .map(|(arg, _)| ident(arg))
        .collect::<Vec<_>>();

    FieldTokens {
        name,
        args,
        field_type,
        field_method,
        params,
        description,
        type_type,
        inner_type,
    }
}

fn argument_to_name_and_rust_type(arg: InputValue, out: &Output) -> (Name, TokenStream) {
    // TODO: use
    //   position
    //   description
    //   default_value
    //   directives

    if let Some(_) = arg.default_value {
        todo!("default value");
    }

    let arg_name = arg.name.to_snake_case();
    let (arg_type, _ty) = gen_field_type(arg.value_type, &FieldTypeDestination::Argument, out);
    (arg_name, arg_type)
}

enum FieldTypeDestination {
    Argument,
    Return(Attributes),
}

fn gen_field_type(
    field_type: Type,
    destination: &FieldTypeDestination,
    out: &Output,
) -> (TokenStream, TypeType) {
    let field_type = NullableType::from_type(field_type);
    let (tokens, ty) = gen_nullable_field_type(field_type, out);

    match (destination, ty) {
        (FieldTypeDestination::Return(attrs), ref ty) => match attrs.ownership() {
            Ownership::Owned => (tokens, *ty),
            Ownership::Borrowed => (quote! { &#tokens }, *ty),
        },

        (FieldTypeDestination::Argument, ty @ TypeType::Scalar) => (tokens, ty),
        (FieldTypeDestination::Argument, ty @ TypeType::Type) => (tokens, ty),
    }
}

fn gen_nullable_field_type(field_type: NullableType, out: &Output) -> (TokenStream, TypeType) {
    use crate::nullable_type::NullableType::*;

    match field_type {
        NamedType(name) => graphql_scalar_type_to_rust_type(name, &out),
        ListType(item_type) => {
            let (item_type, ty) = gen_nullable_field_type(*item_type, &out);
            (quote! { Vec<#item_type> }, ty)
        }
        NullableType(item_type) => {
            let (item_type, ty) = gen_nullable_field_type(*item_type, &out);
            (quote! { Option<#item_type> }, ty)
        }
    }
}

fn gen_with<F, T>(f: F, ts: Vec<T>, other: &Output) -> TokenStream
where
    F: Fn(T, &mut Output),
{
    let mut acc = other.clone_without_tokens();
    for t in ts {
        f(t, &mut acc);
    }
    acc.tokens().into_iter().collect::<TokenStream>()
}

#[derive(Debug, Eq, PartialEq)]
enum Attribute {
    Ownership(Ownership),
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
enum Ownership {
    Borrowed,
    Owned,
}

#[derive(Debug, Eq, PartialEq)]
struct Attributes {
    list: Vec<Attribute>,
}

impl std::default::Default for Attributes {
    fn default() -> Self {
        Attributes { list: Vec::new() }
    }
}

impl Attributes {
    fn ownership(&self) -> Ownership {
        for attr in &self.list {
            match attr {
                Attribute::Ownership(x) => return *x,
                _ => {}
            }
        }

        Ownership::Borrowed
    }
}

fn parse_attributes(desc: &str) -> Attributes {
    let attrs = desc
        .lines()
        .filter_map(|line| parse_attributes_line(line))
        .collect();
    Attributes { list: attrs }
}

lazy_static! {
    static ref ATTRIBUTE_PATTERN: Regex =
        Regex::new(r"\s*#\[(?P<key>\w+)\((?P<value>\w+)\)\]").unwrap();
}

fn parse_attributes_line(line: &str) -> Option<Attribute> {
    let caps = ATTRIBUTE_PATTERN.captures(line)?;
    let key = caps.name("key")?.as_str();
    let value = caps.name("value")?.as_str();

    let attr = match key {
        "ownership" => {
            let value = match value {
                "borrowed" => Ownership::Borrowed,
                "owned" => Ownership::Owned,
                _ => panic!("Unsupported attribute value '{}' for key '{}'", value, key),
            };
            Attribute::Ownership(value)
        }
        _ => panic!("Unsupported attribute key '{}'", key),
    };

    Some(attr)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_descriptions_for_attributes() {
        let desc = r#"
        Comment

        #[ownership(borrowed)]
        "#;
        let attributes = parse_attributes(desc);
        assert_eq!(attributes.ownership(), Ownership::Borrowed);

        let desc = r#"
        Comment

        #[ownership(owned)]
        "#;
        let attributes = parse_attributes(desc);
        assert_eq!(attributes.ownership(), Ownership::Owned);

        let desc = r#"
        Comment
        "#;
        let attributes = parse_attributes(desc);
        assert_eq!(attributes.ownership(), Ownership::Borrowed);
    }
}
