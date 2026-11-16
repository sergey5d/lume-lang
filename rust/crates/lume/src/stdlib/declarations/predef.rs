use super::super::schema::*;
use crate::ast::Item;

pub(super) fn items() -> Vec<Item> {
    let mut items = vec![
        function(
            "panic",
            vec![],
            vec![vararg("value", ty("Any"))],
            ty("Never"),
        ),
        function(
            "assert",
            vec![],
            vec![
                param("condition", ty("Bool")),
                default_param("message", ty("Str"), string("assert condition was false")),
            ],
            ty("Unit"),
        ),
        function(
            "identity",
            vec![type_param("T")],
            vec![param("value", ty("T"))],
            ty("T"),
        ),
        function(
            "ensure",
            vec![type_param("E")],
            vec![
                param("condition", ty("Bool")),
                param("error", callback(vec![], ty("E"))),
            ],
            result(ty("Unit"), ty("E")),
        ),
    ];
    items.extend(
        super::printer_type::print_methods()
            .into_iter()
            .map(|method| {
                function(
                    &method.name,
                    method.type_params,
                    method.params,
                    method.return_type.expect("print return type"),
                )
            }),
    );
    items
}
