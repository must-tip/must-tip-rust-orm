use lex_developer_acceptance::{Project, ProjectProfile};
use lex_framework::db_controller::{
    self as orm, BooleanField, DecimalField, Entity, JsonField, TextField, UUIDField,
};
use serde_json::json;

// Each backend executes the same application story through concrete public managers.
macro_rules! relation_story {
    ($projects:expr,$profiles:expr)=>{{
        let projects=$projects; let profiles=$profiles;
        let alpha=UUIDField::new(); let beta=UUIDField::new();
        let project=projects.queryset().for_tenant(alpha.clone()).create(&Project { id:UUIDField::new(),tenant_id:alpha.clone(),title:TextField::new("Alpha project"),deleted_at:None }).await.unwrap();
        let other=projects.queryset().for_tenant(beta.clone()).create(&Project { id:UUIDField::new(),tenant_id:beta.clone(),title:TextField::new("Beta secret"),deleted_at:None }).await.unwrap();
        let exact="99999999999999999.123456789";
        for (tenant,parent) in [(&alpha,&project),(&alpha,&other),(&beta,&project)] {
            profiles.queryset().for_tenant(tenant.clone()).create(&ProjectProfile { id:UUIDField::new(),tenant_id:tenant.clone(),project_id:parent.id.clone(),project:None,active:BooleanField::new(true),metadata:JsonField::new(json!({"nested":{"visible":"世界"},"values":[1,true,null]})),amount:DecimalField::from_str(exact).unwrap() }).await.unwrap();
        }
        let ordinary=profiles.queryset().for_tenant(alpha.clone()).all().await.unwrap();
        assert_eq!(ordinary.len(),2);
        assert!(ordinary.iter().all(|row|row.project.is_none()),"relations remain unloaded until explicitly requested");
        let relationship=ProjectProfile::relations().into_iter().find(|relation|relation.field_name=="project").unwrap();
        let joined=profiles.queryset().for_tenant(alpha.clone()).select_related::<Project>(&relationship).await.unwrap();
        assert_eq!(joined.iter().filter(|row|row.related.is_some()).count(),1);
        assert!(joined.iter().filter_map(|row|row.related.as_ref()).all(|row|row.title.as_str()=="Alpha project"));
        let reverse=orm::RelationSpec { field_name:"profile",target_table:ProjectProfile::table_name(),local_column:"id",remote_column:"project_id",cardinality:orm::RelationCardinality::One,nullable:true,related_name:None,on_delete:orm::OnDelete::Restrict };
        let eager=projects.queryset().for_tenant(alpha.clone()).select_related::<ProjectProfile>(&reverse).await.unwrap();
        assert_eq!(eager.len(),1); let profile=eager[0].related.as_ref().unwrap();
        assert_eq!(profile.tenant_id.0,alpha.0); assert!(profile.active.0);
        assert_eq!(profile.metadata.0["nested"]["visible"],"世界");
        assert_eq!(profile.amount.value().to_string(),exact,"eager JSON hydration must preserve exact decimals");
        let prefetched=projects.queryset().for_tenant(alpha.clone()).prefetch_related::<ProjectProfile>(&reverse).await.unwrap();
        assert_eq!(prefetched.parents.len(),1);
        let related=prefetched.related_for(&prefetched.parents[0]).unwrap();
        assert_eq!(related.len(),1); assert_eq!(related[0].tenant_id.0,alpha.0);
    }};
}
#[tokio::test]
async fn sqlite_eager_relations_preserve_types_and_cannot_load_another_tenants_records() {
    let pool = orm::sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let projects = Project::objects_sqlite_auto(pool.clone()).await.unwrap();
    let profiles = ProjectProfile::objects_sqlite_auto(pool).await.unwrap();
    relation_story!(projects, profiles);
}
#[cfg(feature = "postgres-integration")]
#[tokio::test]
async fn postgres_eager_relations_preserve_types_and_cannot_load_another_tenants_records() {
    assert_eq!(
        std::env::var("CONFIRM_DISPOSABLE_DATABASE").as_deref(),
        Ok("yes")
    );
    let url = std::env::var("TEST_DATABASE_URL").expect("dedicated test database required");
    let admin = orm::sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("lex_relation_consumer_{}", UUIDField::new().0.simple());
    orm::sqlx::query(orm::sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let setting = format!("SET search_path TO {schema}");
    let pool = orm::sqlx::postgres::PgPoolOptions::new()
        .after_connect(move |connection, _| {
            let sql = setting.clone();
            Box::pin(async move {
                orm::sqlx::query(orm::sqlx::AssertSqlSafe(sql))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    for ddl in [
        orm::generate_create_table_sql::<Project>(),
        orm::generate_create_table_sql::<ProjectProfile>(),
    ] {
        orm::sqlx::raw_sql(orm::sqlx::AssertSqlSafe(ddl))
            .execute(&pool)
            .await
            .unwrap();
    }
    relation_story!(
        Project::objects(pool.clone()),
        ProjectProfile::objects(pool.clone())
    );
    pool.close().await;
    orm::sqlx::query(orm::sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA {schema} CASCADE"
    )))
    .execute(&admin)
    .await
    .unwrap();
}
