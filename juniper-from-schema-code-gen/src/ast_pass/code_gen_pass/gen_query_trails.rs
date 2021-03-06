use super::{ident, type_name, CodeGenPass, EmitError, FieldTypeDestination, TypeKind};
use crate::ast_pass::{error::ErrorKind, schema_visitor::SchemaVisitor};
use graphql_parser::schema::*;
use heck::{CamelCase, MixedCase, SnakeCase};
use proc_macro2::TokenStream;
use quote::quote;
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
};
use syn::Ident;

struct QueryTrailCodeGenPass<'pass, 'doc> {
    pass: &'pass mut CodeGenPass<'doc>,
    fields_map: HashMap<&'doc String, Vec<&'doc Field>>,
}

impl<'doc> CodeGenPass<'doc> {
    pub fn gen_query_trails(&mut self, doc: &'doc Document) {
        let original_tokens = std::mem::replace(&mut self.tokens, quote! {});

        let fields_map = build_fields_map(doc);

        let mut query_trail_pass = QueryTrailCodeGenPass {
            pass: self,
            fields_map,
        };
        query_trail_pass.gen_query_trail();
        query_trail_pass.gen_from_default_scalar_value();
        query_trail_pass.gen_from_look_ahead_value();
        query_trail_pass.visit_document(doc);

        let query_trail_tokens = &self.tokens;

        self.tokens = quote! {
            pub use juniper_from_schema::{Walked, NotWalked, QueryTrail};
            pub use self::query_trails::*;

            #original_tokens

            /// `QueryTrail` extension traits specific to the GraphQL schema
            ///
            /// Generated by `juniper-from-schema`.
            pub mod query_trails {
                #![allow(unused_imports, dead_code, missing_docs)]

                use super::*;

                #query_trail_tokens
            }
        };
    }
}

impl<'pass, 'doc> QueryTrailCodeGenPass<'pass, 'doc> {
    fn gen_query_trail(&mut self) {
        self.pass.extend(quote! {
            use juniper_from_schema::{Walked, NotWalked, QueryTrail};

            /// Convert from one type of `QueryTrail` to another. Used for converting interface and
            /// union trails into concrete subtypes.
            ///
            /// This trait cannot live in juniper-from-schema itself because then we wouldn't be
            /// able to implement it for `QueryTrail` in the user's code. That would result in
            /// orphan instances.
            ///
            /// Generated by `juniper-from-schema`.
            pub trait DowncastQueryTrail<'a, T> {
                /// Perform the downcast.
                ///
                /// Generated by juniper-from-schema.
                fn downcast(self) -> QueryTrail<'a, T, Walked>;
            }
        })
    }

    fn gen_from_default_scalar_value(&mut self) {
        self.pass.extend(quote! {
            /// Convert a `juniper::DefaultScalarValue` into a concrete value.
            ///
            /// This is used for `QueryTrail`.
            ///
            /// Generated by `juniper-from-schema`.
            pub(super) trait FromDefaultScalarValue<T> {
                /// Perform the conversion.
                fn from(self) -> T;
            }
        });

        let gen_impl = |to: &str, variant: &str| {
            let to = ident(to);
            let variant = ident(variant);
            quote! {
                impl<'a, 'b> FromDefaultScalarValue<#to> for &'a &'b juniper::DefaultScalarValue {
                    fn from(self) -> #to {
                        match self {
                            juniper::DefaultScalarValue::#variant(x) => x.to_owned(),
                            other => {
                                match other {
                                    juniper::DefaultScalarValue::Int(_) => panic!(
                                        "Failed converting scalar value. Expected `{}` got `Int`",
                                        stringify!(#to),
                                    ),
                                    juniper::DefaultScalarValue::String(_) => panic!(
                                        "Failed converting scalar value. Expected `{}` got `String`",
                                        stringify!(#to),
                                    ),
                                    juniper::DefaultScalarValue::Float(_) => panic!(
                                        "Failed converting scalar value. Expected `{}` got `Float`",
                                        stringify!(#to),
                                    ),
                                    juniper::DefaultScalarValue::Boolean(_) => panic!(
                                        "Failed converting scalar value. Expected `{}` got `Boolean`",
                                        stringify!(#to),
                                    ),
                                }
                            }
                        }
                    }
                }
            }
        };

        self.pass.extend(gen_impl("i32", "Int"));
        self.pass.extend(gen_impl("String", "String"));
        self.pass.extend(gen_impl("f64", "Float"));
        self.pass.extend(gen_impl("bool", "Boolean"));

        self.pass.extend(quote! {
            impl<'a, 'b, T> FromDefaultScalarValue<Option<T>> for &'a &'b juniper::DefaultScalarValue
            where
                &'a &'b juniper::DefaultScalarValue: FromDefaultScalarValue<T>,
            {
                fn from(self) -> Option<T> {
                    Some(self.from())
                }
            }
        });
    }

    fn gen_from_look_ahead_value(&mut self) {
        self.pass.extend(quote! {
            /// Convert a `juniper::LookAheadValue` into a concrete value.
            ///
            /// This is used for `QueryTrail`.
            ///
            /// Generated by `juniper-from-schema`.
            pub(super) trait FromLookAheadValue<T> {
                /// Perform the conversion.
                fn from(self) -> T;
            }
        });

        let gen_scalar_impl = |to: &str| {
            let to = ident(to);
            quote! {
                impl<'a, 'b> FromLookAheadValue<#to>
                    for &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>
                {
                    fn from(self) -> #to {
                        match self {
                            juniper::LookAheadValue::Scalar(scalar) => {
                                FromDefaultScalarValue::from(scalar)
                            },
                            juniper::LookAheadValue::Null => panic!(
                                "Failed converting look ahead value. Expected scalar type got `null`",
                            ),
                            juniper::LookAheadValue::Enum(_) => panic!(
                                "Failed converting look ahead value. Expected scalar type got `enum`",
                            ),
                            juniper::LookAheadValue::List(_) => panic!(
                                "Failed converting look ahead value. Expected scalar type got `list`",
                            ),
                            juniper::LookAheadValue::Object(_) => panic!(
                                "Failed converting look ahead value. Expected scalar type got `object`",
                            ),
                        }
                    }
                }
            }
        };

        self.pass.extend(gen_scalar_impl("i32"));
        self.pass.extend(gen_scalar_impl("String"));
        self.pass.extend(gen_scalar_impl("f64"));
        self.pass.extend(gen_scalar_impl("bool"));

        self.pass.extend(quote! {
            impl<'a, 'b, T> FromLookAheadValue<Option<T>>
                for &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>
            where
                &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>: FromLookAheadValue<T>,
            {
                fn from(self) -> Option<T> {
                    match self {
                        juniper::LookAheadValue::Null => None,
                        other => Some(other.from()),
                    }
                }
            }

            impl<'a, 'b, T> FromLookAheadValue<Vec<T>>
                for &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>
            where
                &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>: FromLookAheadValue<T>,
            {
                fn from(self) -> Vec<T> {
                    match self {
                        juniper::LookAheadValue::List(values) => {
                            values.iter().map(|value| value.from()).collect::<Vec<_>>()
                        },
                        juniper::LookAheadValue::Scalar(_) => panic!(
                            "Failed converting look ahead value. Expected list type got `scalar`",
                        ),
                        juniper::LookAheadValue::Null => panic!(
                            "Failed converting look ahead value. Expected list type got `null`",
                        ),
                        juniper::LookAheadValue::Enum(_) => panic!(
                            "Failed converting look ahead value. Expected list type got `enum`",
                        ),
                        juniper::LookAheadValue::Object(_) => panic!(
                            "Failed converting look ahead value. Expected list type got `object`",
                        ),
                    }
                }
            }

            impl<'a, 'b> FromLookAheadValue<juniper::ID>
                for &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>
            {
                fn from(self) -> juniper::ID {
                    let s = FromLookAheadValue::<String>::from(self);
                    juniper::ID::new(s)
                }
            }
        });

        if self.pass.ast_data.url_scalar_defined() {
            self.pass.extend(quote! {
                impl<'a, 'b> FromLookAheadValue<url::Url>
                    for &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>
                {
                    fn from(self) -> url::Url {
                        let s = FromLookAheadValue::<String>::from(self);
                        match url::Url::parse(&s) {
                            Ok(url) => url,
                            Err(e) => panic!("Error parsing URL: {}", e),
                        }
                    }
                }
            });
        }

        if self.pass.ast_data.uuid_scalar_defined() {
            self.pass.extend(quote! {
                impl<'a, 'b> FromLookAheadValue<uuid::Uuid>
                    for &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>
                {
                    fn from(self) -> uuid::Uuid {
                        let s = FromLookAheadValue::<String>::from(self);
                        match uuid::Uuid::parse_str(&s) {
                            Ok(url) => url,
                            Err(e) => panic!("Error parsing UUID: {}", e),
                        }
                    }
                }
            });
        }

        if self.pass.ast_data.date_scalar_defined() {
            self.pass.extend(quote! {
                impl<'a, 'b> FromLookAheadValue<chrono::NaiveDate>
                    for &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>
                {
                    fn from(self) -> chrono::NaiveDate {
                        let s = FromLookAheadValue::<String>::from(self);
                        match chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
                            Ok(date) => date,
                            Err(e) => {
                                panic!(
                                    "Error parsing NaiveDate. Format used is `%Y-%m-%d`\n{}",
                                    e,
                                )
                            },
                        }
                    }
                }
            });
        }

        if self.pass.ast_data.date_time_scalar_defined() {
            self.pass.extend(quote! {
                impl<'a, 'b> FromLookAheadValue<chrono::DateTime<chrono::Utc>>
                    for &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>
                {
                    fn from(self) -> chrono::DateTime<chrono::Utc> {
                        let s = FromLookAheadValue::<String>::from(self);
                        let parsed = chrono::DateTime::parse_from_rfc3339(&s);
                        match parsed {
                            Ok(date_time) => date_time.into(),
                            Err(e) => {
                                panic!(
                                    "Error parsing DateTime. Format used is RFC 3339 (aka ISO 8601)\n{}",
                                    e,
                                )
                            },
                        }
                    }
                }

                impl<'a, 'b> FromLookAheadValue<chrono::NaiveDateTime>
                    for &'a juniper::LookAheadValue<'b, juniper::DefaultScalarValue>
                {
                    fn from(self) -> chrono::NaiveDateTime {
                        let s = FromLookAheadValue::<String>::from(self);
                        let parsed = chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S");
                        match parsed {
                            Ok(date_time) => date_time.into(),
                            Err(e) => {
                                panic!(
                                    "Error parsing NaiveDateTime. Format used is `%Y-%m-%d %H:%M:%S`\n{}",
                                    e,
                                )
                            },
                        }
                    }
                }
            });
        }
    }

    fn gen_field_walk_methods(&mut self, obj: InternalQueryTrailNode) {
        let name = ident(&obj.name());
        let trait_name = ident(&format!("QueryTrail{}Extensions", obj.name()));
        let args_trait_name = ident(&format!("QueryTrail{}ArgumentsExtensions", obj.name()));
        let fields = obj.fields();

        let mut method_signatures = vec![];
        let mut method_implementations = vec![];

        let mut argument_signatures = vec![];
        let mut argument_implementations = vec![];
        let mut argument_types = vec![];

        for field in fields {
            let FieldWalkMethod {
                method_signature,
                method_implementation,
                argument_signature,
                argument_implementation,
                argument_type,
            } = self.gen_field_walk_method(field, &obj);

            method_signatures.push(method_signature);
            method_implementations.push(method_implementation);

            argument_signatures.push(argument_signature);
            argument_implementations.push(argument_implementation);
            argument_types.push(argument_type);
        }

        self.pass.extend(quote! {
            /// Extension trait for `QueryTrail` to inspect incoming queries.
            pub trait #trait_name<'a, K> {
                #(#method_signatures)*
            }

            impl<'a, K> #trait_name<'a, K> for QueryTrail<'a, #name, K> {
                #(#method_implementations)*
            }

            /// Extension trait for `QueryTrail` to inspect incoming query arguments.
            pub trait #args_trait_name<'a> {
                #(#argument_signatures)*
            }

            impl<'a> #args_trait_name<'a> for QueryTrail<'a, #name, juniper_from_schema::Walked> {
                #(#argument_implementations)*
            }

            #(#argument_types)*
        });

        self.gen_conversion_methods(name, obj);
    }

    fn gen_conversion_methods(
        &mut self,
        original_type_name: Ident,
        obj: InternalQueryTrailNode<'_>,
    ) {
        let mut destination_types = vec![];

        match obj {
            InternalQueryTrailNode::Object(_) => {}
            InternalQueryTrailNode::Interface(i) => {
                if let Some(i) = &self.pass.ast_data.get_implementors_of_interface(&i.name) {
                    for interface_implementor_name in *i {
                        let ident = ident(interface_implementor_name);
                        destination_types.push(ident);
                    }
                }
            }
            InternalQueryTrailNode::Union(u, _) => {
                for type_ in &u.types {
                    let ident = ident(type_);
                    destination_types.push(ident);
                }
            }
        }

        for type_ in destination_types {
            self.pass.extend(quote! {
                impl<'a> DowncastQueryTrail<'a, #type_> for &QueryTrail<'a, #original_type_name, Walked> {
                    fn downcast(self) -> QueryTrail<'a, #type_, Walked> {
                        QueryTrail {
                            look_ahead: self.look_ahead,
                            node_type: std::marker::PhantomData,
                            walked: juniper_from_schema::Walked,
                        }
                    }
                }
            });
        }
    }

    fn error_msg_if_field_types_dont_overlap(&mut self, union: &'doc UnionType) {
        let fields_map = &self.fields_map;
        let mut prev: HashMap<&'doc str, (&'doc str, &'doc str)> = HashMap::new();

        for type_b in &union.types {
            if let Some(fields) = fields_map.get(type_b) {
                for field in fields {
                    let field_type_b = type_name(&field.field_type);

                    if let Some((type_a, field_type_a)) = prev.get(&field.name.as_ref()) {
                        if field_type_b != field_type_a {
                            self.pass.emit_non_fatal_error(
                                union.position,
                                ErrorKind::UnionFieldTypeMismatch {
                                    union_name: &union.name,
                                    field_name: &field.name,
                                    type_a: &type_a,
                                    type_b: &type_b,
                                    field_type_a: &field_type_a,
                                    field_type_b: &field_type_b,
                                },
                            );
                        }
                    }

                    prev.insert(&field.name, (type_b, field_type_b));
                }
            }
        }
    }

    fn gen_field_walk_method(
        &mut self,
        field: &Field,
        obj: &InternalQueryTrailNode,
    ) -> FieldWalkMethod {
        let field_type = type_name(&field.field_type);
        let (_, ty) = self
            .pass
            .graphql_scalar_type_to_rust_type(&field_type, field.position);
        let field_type = ident(field_type.clone().to_camel_case());

        match ty {
            TypeKind::Scalar => {
                let name = ident(&field.name.to_snake_case());
                let string_name = &field.name.to_mixed_case();

                let method_signature = quote! {
                    /// Check if a scalar leaf node is queried for
                    ///
                    /// Generated by `juniper-from-schema`.
                    fn #name(&self) -> bool;
                };

                let method_implementation = quote! {
                    fn #name(&self) -> bool {
                        use juniper::LookAheadMethods;

                        self.look_ahead
                            .and_then(|la| la.select_child(#string_name))
                            .is_some()
                    }
                };

                let (argument_signature, argument_implementation, argument_type) =
                    self.gen_args_query_trail(field, &name, obj);

                FieldWalkMethod {
                    method_signature,
                    method_implementation,
                    argument_signature,
                    argument_implementation,
                    argument_type,
                }
            }
            TypeKind::Type => {
                let name = ident(&field.name.to_snake_case());
                let string_name = &field.name.to_mixed_case();

                let method_signature = quote! {
                    /// Walk the trail into a field.
                    ///
                    /// Generated by `juniper-from-schema`.
                    fn #name(&self) -> QueryTrail<'a, #field_type, juniper_from_schema::NotWalked>;
                };

                let method_implementation = quote! {
                    fn #name(&self) -> QueryTrail<'a, #field_type, juniper_from_schema::NotWalked> {
                        use juniper::LookAheadMethods;

                        let child = self.look_ahead.and_then(|la| la.select_child(#string_name));

                        QueryTrail {
                            look_ahead: child,
                            node_type: std::marker::PhantomData,
                            walked: juniper_from_schema::NotWalked,
                        }
                    }
                };

                let (argument_signature, argument_implementation, argument_type) =
                    self.gen_args_query_trail(field, &name, obj);

                FieldWalkMethod {
                    method_signature,
                    method_implementation,
                    argument_signature,
                    argument_implementation,
                    argument_type,
                }
            }
        }
    }

    fn gen_args_query_trail(
        &mut self,
        field: &Field,
        name: &Ident,
        obj: &InternalQueryTrailNode,
    ) -> (TokenStream, TokenStream, TokenStream) {
        let mut argument_signature = quote! {};
        let mut argument_implementation = quote! {};
        let mut argument_type = quote! {};

        let obj_type = ident(&obj.name());

        let args_method_name = ident(&format!("{}_args", name));

        if field.arguments.is_empty() {
            argument_signature.extend(quote! {
                /// Inspect argument in incoming query.
                ///
                /// This field takes no arguments, so therefore it returns `()`.
                fn #args_method_name(&self) -> ();
            });

            argument_implementation.extend(quote! {
                #[allow(missing_docs)]
                #[inline]
                fn #args_method_name(&self) -> () {
                    ()
                }
            });
        } else {
            let args_type_name = ident(&format!(
                "{}{}Args",
                obj.name(),
                name.to_string().to_camel_case()
            ));

            argument_signature.extend(quote! {
                /// Inspect argument in incoming query.
                fn #args_method_name(&'a self) -> #args_type_name<'a>;
            });

            argument_implementation.extend(quote! {
                #[allow(missing_docs)]
                fn #args_method_name(&'a self) -> #args_type_name<'a> {
                    #args_type_name(self)
                }
            });

            let arguments_methods = field
                .arguments
                .iter()
                .map(|input_value| self.gen_argument_look_ahead_methods(input_value, &field.name));

            argument_type.extend(quote! {
                /// This is used for inspecting arguments to a field.
                ///
                /// Generated by `juniper-from-schema`.
                pub struct #args_type_name<'a>(
                    &'a QueryTrail<'a, #obj_type, juniper_from_schema::Walked>
                );

                impl<'a> #args_type_name<'a> {
                    #(#arguments_methods)*
                }
            });
        }

        (argument_signature, argument_implementation, argument_type)
    }

    fn gen_argument_look_ahead_methods(
        &mut self,
        input_value: &InputValue,
        field_name: &str,
    ) -> TokenStream {
        let default_value = input_value.default_value.as_ref().map(|value| {
            self.pass.quote_value(
                &value,
                type_name(&input_value.value_type),
                input_value.position,
            )
        });

        let (field_type, _) = self.pass.gen_field_type(
            &input_value.value_type,
            &FieldTypeDestination::Argument,
            default_value.is_some(),
            input_value.position,
        );

        let name = &input_value.name;
        let ident = ident(name.to_snake_case());

        if let Some(default_value) = default_value {
            quote! {
                #[allow(missing_docs)]
                pub fn #ident(&self) -> #field_type {
                    use juniper::LookAheadMethods;

                    // these `expect`s are fine since these methods you can only obtain
                    // arguments from walked query trails
                    let lh = &self
                        .0
                        .look_ahead
                        .expect("look_ahead")
                        .select_child(#field_name)
                        .expect("select child");

                    let arg = lh.arguments().iter().find(|arg| {
                        arg.name() == #name
                    });

                    if let Some(arg) = arg {
                        let value = arg.value();
                        FromLookAheadValue::<#field_type>::from(value)
                    } else {
                        #default_value
                    }
                }
            }
        } else {
            quote! {
                #[allow(missing_docs)]
                pub fn #ident(&self) -> #field_type {
                    use juniper::LookAheadMethods;

                    // these `expect`s are fine since these methods you can only obtain
                    // arguments from walked query trails
                    let lh = &self
                        .0
                        .look_ahead
                        .expect("look_ahead")
                        .select_child(#field_name)
                        .expect("select child");

                    let arg = lh.arguments().iter().find(|arg| { arg.name() == #name }).expect("no argument with name");
                    let value = arg.value();
                    FromLookAheadValue::<#field_type>::from(value)
                }
            }
        }
    }
}

impl<'pass, 'doc> SchemaVisitor<'doc> for QueryTrailCodeGenPass<'pass, 'doc> {
    fn visit_object_type(&mut self, obj: &'doc ObjectType) {
        self.gen_field_walk_methods(InternalQueryTrailNode::Object(obj));
    }

    fn visit_interface_type(&mut self, interface: &'doc InterfaceType) {
        self.gen_field_walk_methods(InternalQueryTrailNode::Interface(interface))
    }

    fn visit_union_type(&mut self, union: &'doc UnionType) {
        self.error_msg_if_field_types_dont_overlap(union);

        self.gen_field_walk_methods(InternalQueryTrailNode::Union(
            union,
            build_union_fields_set(union, &self.fields_map),
        ))
    }
}

struct FieldWalkMethod {
    method_signature: TokenStream,
    method_implementation: TokenStream,
    argument_signature: TokenStream,
    argument_implementation: TokenStream,
    argument_type: TokenStream,
}

#[derive(Clone, Debug)]
struct HashFieldByName<'a>(&'a Field);

impl<'a> PartialEq for HashFieldByName<'a> {
    fn eq(&self, other: &HashFieldByName) -> bool {
        self.0.name == other.0.name
    }
}

impl<'a> Eq for HashFieldByName<'a> {}

impl<'a> Hash for HashFieldByName<'a> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.name.hash(state);
    }
}

#[derive(Debug)]
enum InternalQueryTrailNode<'a> {
    Object(&'a ObjectType),
    Interface(&'a InterfaceType),
    Union(&'a UnionType, HashSet<HashFieldByName<'a>>),
}

impl<'a> InternalQueryTrailNode<'a> {
    fn name(&self) -> &String {
        match self {
            InternalQueryTrailNode::Object(inner) => &inner.name,
            InternalQueryTrailNode::Interface(inner) => &inner.name,
            InternalQueryTrailNode::Union(inner, _fields) => &inner.name,
        }
    }

    fn fields(&self) -> Vec<&'a Field> {
        match self {
            InternalQueryTrailNode::Object(inner) => inner.fields.iter().collect(),
            InternalQueryTrailNode::Interface(inner) => inner.fields.iter().collect(),
            InternalQueryTrailNode::Union(_inner, fields) => fields
                .iter()
                .map(|hashable_field| hashable_field.0)
                .collect(),
        }
    }
}

fn build_union_fields_set<'d>(
    union: &UnionType,
    fields_map: &HashMap<&'d String, Vec<&'d Field>>,
) -> HashSet<HashFieldByName<'d>> {
    let mut union_fields_set = HashSet::new();

    for type_ in &union.types {
        if let Some(fields) = fields_map.get(type_) {
            for field in fields {
                union_fields_set.insert(HashFieldByName(&field));
            }
        }
    }

    union_fields_set
}

fn build_fields_map(doc: &Document) -> HashMap<&String, Vec<&Field>> {
    let mut map = HashMap::new();

    for def in &doc.definitions {
        if let Definition::TypeDefinition(type_def) = def {
            if let TypeDefinition::Object(obj) = type_def {
                for field in &obj.fields {
                    let entry = map.entry(&obj.name).or_insert_with(|| vec![]);
                    entry.push(field);
                }
            }
        }
    }

    map
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ast_pass::ast_data_pass::AstData;

    #[test]
    fn test_fails_to_generate_query_trail_for_unions_where_fields_dont_overlap() {
        let schema = r#"
            union Entity = User | Company

            type User {
              country: Country!
            }

            type Company {
              country: OtherCountry!
            }

            type Country {
              id: Int!
            }

            type OtherCountry {
              id: Int!
            }
        "#;

        let doc = graphql_parser::parse_schema(&schema).unwrap();
        let ast_data = AstData::new_from_schema_and_doc(&schema, &doc).unwrap();
        let mut out = CodeGenPass {
            tokens: quote! {},
            error_type: crate::parse_input::default_error_type(),
            context_type: crate::parse_input::default_context_type(),
            ast_data,
            errors: std::collections::BTreeSet::new(),
            raw_schema: schema,
        };

        out.gen_query_trails(&doc);

        assert_eq!(1, out.errors.len());
    }
}
