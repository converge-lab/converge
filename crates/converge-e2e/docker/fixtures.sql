insert into users (id, provider, subject, handle, name)
values ('00000000-0000-0000-0000-00000000e2e0', 'local', 'admin', 'admin', 'Admin');

insert into tokens (id, user_id, hash, label)
values (
    '00000000-0000-0000-0000-00000000e2e1',
    '00000000-0000-0000-0000-00000000e2e0',
    encode(
        sha256(convert_to(
            'cvg_0000000000000000000000000000000000000000000000000000000000000e2e',
            'UTF8'
        )),
        'hex'
    ),
    'e2e'
);
