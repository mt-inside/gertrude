#!/usr/bin/env bash

grpcurl -plaintext -import-path ./api/proto/admin/v1/ -proto plugins.proto -d '{}' '[::1]:50051' admin.v1.PluginsService/List
