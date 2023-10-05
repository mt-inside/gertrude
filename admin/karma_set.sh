#!/usr/bin/env bash

term="${1:?need a term}"
value="${2:?need a value}"

grpcurl -plaintext -import-path ./idl/api/proto/admin/v1/ -proto karma.proto -d '{"term": "'${term}'", "value": '${value}'}' '[::1]:50051' admin.v1.KarmaService/Set
